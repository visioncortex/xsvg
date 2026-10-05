//! No authored attribute value can escape the attribute it is written into.
//!
//! The output contract is the static SVG subset: no script, no event handlers. The
//! serializer strips `on*` attributes an author writes — but an emitter that formats
//! an authored *value* into its output without escaping it lets that value close the
//! attribute and open new ones, so `preserveAspectRatio='none" onload="…'` came out
//! as a live `onload`. That shape of bug cannot be found by reading: the emitters
//! that do it are scattered, multi-line, and look like every safe one beside them.
//!
//! So this goes looking. For every sample in the dataset, one attribute at a time,
//! it appends a quote-breaking payload to the value, compiles, and requires the
//! output to be well-formed XML carrying no attribute the payload smuggled in. A
//! new emitter that forgets to escape fails here, whichever element it serves.

use std::path::{Path, PathBuf};
use xsvg_core::test_doubles::{BoxOutliner, BoxShaper, Mono};
use xsvg_core::{compile_linked_impl, Resolver};

/// Closes the attribute it lands in, then opens one the static subset forbids.
const PAYLOAD: &str = "&quot; onxss=&quot;1";

fn dataset() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dataset")
}

/// Cross-file links resolve against the dataset, so linked emission — where the
/// `preserveAspectRatio` injection lived — is exercised, not degraded to a marker.
struct Dataset;
impl Resolver for Dataset {
    fn resolve(&self, _base: &str, href: &str) -> Option<(String, String)> {
        let file = href.split('#').next()?;
        let src = std::fs::read_to_string(dataset().join(file)).ok()?;
        Some((file.to_string(), src))
    }
}

/// Every attribute *value* span in a document, in document order, each with the key
/// it is deduplicated on (see the probe loop).
fn value_spans(src: &str) -> Vec<(String, String, std::ops::Range<usize>)> {
    let Ok(doc) = roxmltree::Document::parse(src) else {
        return Vec::new();
    };
    doc.descendants()
        .filter(|n| n.is_element())
        .flat_map(|n| {
            // The element's *keyword* attributes, coordinates left out: `list="number"`
            // or a token's `name` decide which emitter a value reaches, while two
            // otherwise identical elements at different positions do not.
            let mut context: Vec<String> = n
                .attributes()
                .filter(|a| {
                    a.value().parse::<f64>().is_err() && !a.value().contains(char::is_numeric)
                })
                .map(|a| format!("{}={}", a.name(), a.value()))
                .collect();
            context.sort();
            let tag = n.tag_name().name().to_string();
            n.attributes()
                .map(|a| {
                    let what = format!("<{tag}> {}", a.name());
                    let key = format!("{what} | {}", context.join(" "));
                    (what, key, a.range_value())
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Why `out` shows an escaped value, or `None` when it is clean.
fn escape_failure(out: &str) -> Option<String> {
    let doc = match roxmltree::Document::parse(out) {
        Ok(d) => d,
        Err(e) => return Some(format!("output is not well-formed XML: {e}")),
    };
    doc.descendants().filter(|n| n.is_element()).find_map(|n| {
        n.attributes()
            .find(|a| a.name().starts_with("onxss"))
            .map(|a| {
                format!(
                    "<{} {}=\"{}\"> appeared in the output",
                    n.tag_name().name(),
                    a.name(),
                    a.value()
                )
            })
    })
}

#[test]
fn no_authored_value_escapes_its_attribute() {
    let mut files: Vec<_> = std::fs::read_dir(dataset())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "xsvg"))
        .collect();
    files.sort();
    assert!(
        files.len() > 40,
        "dataset not found at {}",
        dataset().display()
    );

    let mut failures = Vec::new();
    let mut probed = 0;
    for path in &files {
        let src = std::fs::read_to_string(path).unwrap();
        let base = path.file_name().unwrap().to_string_lossy().to_string();
        // One probe per (element, attribute, keyword context) per file. The same pair
        // repeats dozens of times in a sample through the same code, so probing every
        // occurrence costs ~10x for nothing; but one attribute can reach different
        // sinks depending on its element's keywords (`marker-fill` lands in a <tspan>
        // in a numbered list and a <circle> in a bulleted one), so those stay apart.
        let mut seen = std::collections::HashSet::new();
        for (what, key, span) in value_spans(&src) {
            if !seen.insert(key) {
                continue;
            }
            let probe = format!("{}{PAYLOAD}{}", &src[..span.end], &src[span.end..]);
            let Ok(out) = compile_linked_impl(
                &probe,
                "balanced",
                false,
                &Mono,
                &BoxShaper,
                &BoxOutliner,
                &Dataset,
                &base,
            ) else {
                continue; // the probe made the document itself unparseable — nothing emitted
            };
            probed += 1;
            if let Some(why) = escape_failure(&out) {
                failures.push(format!("{base}: payload in {what} → {why}"));
            }
        }
    }
    assert!(
        probed > 500,
        "only {probed} probes ran — the harness is not reaching the dataset"
    );
    assert!(
        failures.is_empty(),
        "{} authored value(s) escaped their attribute over {probed} probes:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}
