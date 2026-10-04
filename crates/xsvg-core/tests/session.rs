//! A compile session emits exactly what the pure entries emit (docs/Compile
//! Session.md §6, "Equality").
//!
//! The session exists so an editor can compile many fragments of one document on
//! one parse; its contract is that this changes nothing but the work done. So, for
//! every sample in the dataset and every fragment in it, the session's answer is
//! compared byte for byte with the pure entry's — the whole compile, each fragment,
//! the dependents of each fragment and of all of them together, the caret geometry
//! of every textbox, and the span tables — in plain, source-mapped and keyed modes.

use std::path::{Path, PathBuf};
use xsvg_core::test_doubles::{BoxOutliner, BoxShaper, Mono};
use xsvg_core::{
    compile_fragment_seams_impl, compile_seams_impl, dependents_multi_impl, fragment_range_impl,
    node_spans_impl, text_layout_impl, CompileSession, NodeKeys, Resolver, Seams,
};

fn dataset() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dataset")
}

/// Cross-file links resolve against the dataset, so linked emission is compared too.
struct Dataset;
impl Resolver for Dataset {
    fn resolve(&self, _base: &str, href: &str) -> Option<(String, String)> {
        let file = href.split('#').next()?;
        let src = std::fs::read_to_string(dataset().join(file)).ok()?;
        Some((file.to_string(), src))
    }
}

/// A host that keys every element by its byte start — stable for one text.
struct SpanKeys;
impl NodeKeys for SpanKeys {
    fn key(&self, byte_start: usize) -> Option<String> {
        Some(format!("k{byte_start}"))
    }
}

fn samples() -> Vec<(String, String)> {
    let mut files: Vec<_> = std::fs::read_dir(dataset())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "xsvg"))
        .collect();
    files.sort();
    assert!(files.len() > 40, "dataset not found at {}", dataset().display());
    files
        .into_iter()
        .map(|p| {
            let base = p.file_name().unwrap().to_string_lossy().to_string();
            (base, std::fs::read_to_string(&p).unwrap())
        })
        .collect()
}

/// Byte starts of the top-level elements, and of every `<x:textbox>`.
fn offsets(src: &str) -> (Vec<usize>, Vec<usize>) {
    let doc = roxmltree::Document::parse(src).unwrap();
    let tops = doc
        .root_element()
        .children()
        .filter(|c| c.is_element())
        .map(|c| c.range().start)
        .collect();
    let boxes = doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "textbox")
        .map(|n| n.range().start)
        .collect();
    (tops, boxes)
}

#[test]
fn a_session_emits_what_the_pure_entries_emit() {
    let mut fragments = 0;
    for (base, src) in samples() {
        let (tops, boxes) = offsets(&src);
        for (sourcemap, keyed) in [(false, false), (true, false), (true, true)] {
            let seams = Seams::new(&Mono, &BoxShaper, &BoxOutliner).with_resolver(&Dataset);
            let seams = if keyed { seams.with_keys(&SpanKeys) } else { seams };
            let mode = format!("{base} sourcemap={sourcemap} keyed={keyed}");

            let session = CompileSession::new(&src, "balanced", sourcemap, &base).unwrap();
            assert_eq!(
                session.compile(&seams),
                compile_seams_impl(&src, "balanced", sourcemap, &base, &seams),
                "{mode}: whole compile"
            );
            for &off in &tops {
                assert_eq!(
                    session.compile_fragment(off, &seams),
                    compile_fragment_seams_impl(&src, "balanced", sourcemap, off, &base, &seams),
                    "{mode}: fragment at {off}"
                );
                fragments += 1;
            }
        }

        let session = CompileSession::new(&src, "balanced", false, "").unwrap();
        for &off in &tops {
            assert_eq!(
                session.dependents(&[off]),
                dependents_multi_impl(&src, &[off]),
                "{base}: dependents of {off}"
            );
            assert_eq!(
                session.fragment_range(off),
                fragment_range_impl(&src, off),
                "{base}: fragment range at {off}"
            );
        }
        assert_eq!(
            session.dependents(&tops),
            dependents_multi_impl(&src, &tops),
            "{base}: dependents of every fragment"
        );
        assert_eq!(session.node_spans(), node_spans_impl(&src), "{base}: node spans");
        let seams = Seams::new(&Mono, &BoxShaper, &BoxOutliner);
        for &off in &boxes {
            assert_eq!(
                session.text_layout(off, &seams),
                text_layout_impl(&src, off, "balanced", &Mono, &BoxShaper, &BoxOutliner),
                "{base}: text layout of the textbox at {off}"
            );
        }
    }
    assert!(fragments > 300, "only {fragments} fragments compared — the corpus is not being reached");
}

#[test]
fn a_session_fails_where_the_pure_entries_fail() {
    let seams = Seams::new(&Mono, &BoxShaper, &BoxOutliner);
    // malformed XML: the same message
    let bad = "<svg><rect></svg>";
    assert_eq!(
        CompileSession::new(bad, "balanced", false, "").err(),
        compile_seams_impl(bad, "balanced", false, "", &seams).err()
    );
    // pathological nesting: refused before the parser sees it, same message
    let deep = format!("<svg>{}{}</svg>", "<g>".repeat(600), "</g>".repeat(600));
    assert_eq!(
        CompileSession::new(&deep, "balanced", false, "").err(),
        compile_seams_impl(&deep, "balanced", false, "", &seams).err()
    );
    // an offset outside every top-level element: the same message
    let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="1" height="1"/></svg>"#;
    let session = CompileSession::new(src, "balanced", false, "").unwrap();
    assert_eq!(
        session.compile_fragment(2, &seams),
        compile_fragment_seams_impl(src, "balanced", false, 2, "", &seams)
    );
}

/// A session may be built on one thread and used on another (an app that parses
/// off the UI thread); it holds no thread-bound state. It is not `Sync` by design —
/// the seams are passed per call precisely so hosts with single-threaded seams can
/// still have a `Send` session.
#[test]
fn a_session_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<CompileSession<'static>>();
}
