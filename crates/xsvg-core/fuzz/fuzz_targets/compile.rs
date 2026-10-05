//! Parse, then compile, and never panic.
//!
//! Every entry an embedder can reach with a document is driven over one input:
//! the whole compile, every fragment, the dependents of every fragment, the caret
//! geometry of every element (a non-textbox answers `None`), and the span
//! tables — in keyed, source-mapped mode, which exercises the most emitters.
//! Malformed input must come back as `Err`, never as a panic or a hang: the
//! viewer and the VS Code extension open untrusted `.xsvg` files, and
//! docs/Sandbox.md's output rules assume the compiler survives its input.
//!
//! The platform is the deterministic test doubles — the fuzzer is after the
//! compiler's own faults, not a font's.
#![no_main]

use libfuzzer_sys::fuzz_target;
use xsvg_core::test_doubles::{BoxOutliner, BoxShaper, Mono};
use xsvg_core::{NodeKeys, Platform};

/// Every element keyed by its byte start, so keyed emission is reached.
struct SpanKeys;
impl NodeKeys for SpanKeys {
    fn key(&self, byte_start: usize) -> Option<String> {
        Some(format!("k{byte_start}"))
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };
    let platform = Platform::new(Mono, BoxShaper, BoxOutliner).with_keys(SpanKeys);
    let Ok(session) = platform.session(input, "balanced", true, "") else {
        return; // a parse error is an answer, not a failure
    };
    let _ = session.compile();
    let spans = session.node_spans();
    let mut tops = Vec::new();
    for &(start, _) in &spans {
        if let Some(range) = session.fragment_range(start) {
            if range.0 == start {
                tops.push(start);
            }
        }
        let _ = session.compile_fragment(start);
        let _ = session.text_layout(start);
    }
    let _ = session.dependents(&tops);
});
