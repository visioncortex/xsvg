//! A compile session emits exactly what the pure entries emit, and the measurement
//! cache changes nothing but the calls that reach the adapter (docs/Compile
//! Session.md §6, "Equality" and "Caches are transparent").
//!
//! The session exists so an editor can compile many fragments of one document on
//! one parse; its contract is that this changes nothing but the work done. So, for
//! every sample in the dataset and every fragment in it, the session's answer is
//! compared byte for byte with the pure entry's — the whole compile, each fragment,
//! the dependents of each fragment and of all of them together, the caret geometry
//! of every textbox, and the span tables — in plain, source-mapped and keyed modes,
//! with and without the cache. The auto traits are deliberately not pinned (§8).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use xsvg_core::test_doubles::{BoxOutliner, BoxShaper, Mono};
use xsvg_core::{
    compile_fragment_platform_impl, compile_platform_impl, dependents_multi_impl,
    fragment_range_impl, node_spans_impl, text_layout_impl, FontMetrics, MeasureCache, Measurer,
    NodeKeys, Platform, Resolver, TextStyle,
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

/// A host that keys every element by its byte start — stable for one text — or,
/// when off, the inactive byte-span default.
struct SpanKeys(bool);
impl NodeKeys for SpanKeys {
    fn key(&self, byte_start: usize) -> Option<String> {
        self.0.then(|| format!("k{byte_start}"))
    }
    fn active(&self) -> bool {
        self.0
    }
}

fn samples() -> Vec<(String, String)> {
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

/// The dataset platform in one of its modes. The measurer is boxed so the cached
/// and plain platforms share a type; `Box<dyn Measurer>` is itself a `Measurer`.
fn platform(
    keyed: bool,
    cached: bool,
) -> Platform<Box<dyn Measurer>, BoxShaper, BoxOutliner, Dataset, SpanKeys> {
    let m: Box<dyn Measurer> = if cached {
        Box::new(MeasureCache::new(Mono))
    } else {
        Box::new(Mono)
    };
    Platform::new(m, BoxShaper, BoxOutliner)
        .with_resolver(Dataset)
        .with_keys(SpanKeys(keyed))
}

#[test]
fn a_session_emits_what_the_pure_entries_emit() {
    let mut fragments = 0;
    for (base, src) in samples() {
        let (tops, boxes) = offsets(&src);
        for (sourcemap, keyed) in [(false, false), (true, false), (true, true)] {
            let platform = platform(keyed, false);
            let mode = format!("{base} sourcemap={sourcemap} keyed={keyed}");

            let session = platform
                .session(&src, "balanced", sourcemap, &base)
                .unwrap();
            assert_eq!(
                session.compile(),
                compile_platform_impl(&src, "balanced", sourcemap, &base, &platform),
                "{mode}: whole compile"
            );
            for &off in &tops {
                assert_eq!(
                    session.compile_fragment(off),
                    compile_fragment_platform_impl(
                        &src, "balanced", sourcemap, off, &base, &platform
                    ),
                    "{mode}: fragment at {off}"
                );
                fragments += 1;
            }
        }

        let platform = Platform::new(Mono, BoxShaper, BoxOutliner);
        let session = platform.session(&src, "balanced", false, "").unwrap();
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
        assert_eq!(
            session.node_spans(),
            node_spans_impl(&src),
            "{base}: node spans"
        );
        for &off in &boxes {
            assert_eq!(
                session.text_layout(off),
                text_layout_impl(&src, off, "balanced", &Mono, &BoxShaper, &BoxOutliner),
                "{base}: text layout of the textbox at {off}"
            );
        }
    }
    assert!(
        fragments > 300,
        "only {fragments} fragments compared — the corpus is not being reached"
    );
}

#[test]
fn the_measure_cache_changes_no_output() {
    // One cached platform across the whole corpus, as an editor would hold it: the
    // second sample onward is served partly from entries the earlier ones filled.
    let cached = platform(true, true);
    let plain = platform(true, false);
    for (base, src) in samples() {
        let (tops, boxes) = offsets(&src);
        let a = cached.session(&src, "balanced", true, &base).unwrap();
        let b = plain.session(&src, "balanced", true, &base).unwrap();
        assert_eq!(a.compile(), b.compile(), "{base}: whole compile");
        for &off in &tops {
            assert_eq!(
                a.compile_fragment(off),
                b.compile_fragment(off),
                "{base}: fragment at {off}"
            );
        }
        for &off in &boxes {
            assert_eq!(
                a.text_layout(off),
                b.text_layout(off),
                "{base}: layout at {off}"
            );
        }
    }
}

/// `Mono` that counts the calls reaching it.
struct Counting(AtomicUsize);
impl Measurer for Counting {
    fn measure(&self, text: &str, style: &TextStyle, size: f64) -> f64 {
        self.0.fetch_add(1, Ordering::Relaxed);
        Mono.measure(text, style, size)
    }
    fn font_metrics(&self, style: &TextStyle, size: f64) -> FontMetrics {
        self.0.fetch_add(1, Ordering::Relaxed);
        Mono.font_metrics(style, size)
    }
}

#[test]
fn a_second_flush_measures_nothing_the_first_one_did() {
    let src = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">
<x:textbox x="0" y="0" width="80" height="60" font-size="16">alpha beta gamma delta</x:textbox>
<x:textbox x="0" y="100" width="60" height="30" font-size="20" fit="shrink">epsilon zeta</x:textbox>
</svg>"#;
    let cache = MeasureCache::new(Counting(AtomicUsize::new(0)));
    let platform = Platform::new(&cache, BoxShaper, BoxOutliner);

    // flush 1: everything is measured once
    let first = platform
        .session(src, "balanced", true, "")
        .unwrap()
        .compile()
        .unwrap();
    let measured = cache.inner().0.load(Ordering::Relaxed);
    assert!(measured > 0, "the first flush reaches the adapter");

    // flush 2 over the same text, as a keystroke elsewhere in the document would
    // cause: the same output, and not one more call to the adapter
    let second = platform
        .session(src, "balanced", true, "")
        .unwrap()
        .compile()
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(
        cache.inner().0.load(Ordering::Relaxed),
        measured,
        "every measurement was a hit"
    );

    // a new word costs exactly what is new; its font's metrics are already known
    let edited = src.replace("epsilon zeta", "epsilon zeta eta");
    platform
        .session(&edited, "balanced", true, "")
        .unwrap()
        .compile()
        .unwrap();
    let added = cache.inner().0.load(Ordering::Relaxed) - measured;
    assert!(
        added >= 1 && added <= 4,
        "one new word, plus at most the lines it changed: {added}"
    );
}

#[test]
fn a_session_fails_where_the_pure_entries_fail() {
    let platform = Platform::new(Mono, BoxShaper, BoxOutliner);
    // malformed XML: the same message
    let bad = "<svg><rect></svg>";
    assert_eq!(
        platform.session(bad, "balanced", false, "").err(),
        compile_platform_impl(bad, "balanced", false, "", &platform).err()
    );
    // pathological nesting: refused before the parser sees it, same message
    let deep = format!("<svg>{}{}</svg>", "<g>".repeat(600), "</g>".repeat(600));
    assert_eq!(
        platform.session(&deep, "balanced", false, "").err(),
        compile_platform_impl(&deep, "balanced", false, "", &platform).err()
    );
    // an offset outside every top-level element: the same message
    let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="1" height="1"/></svg>"#;
    let session = platform.session(src, "balanced", false, "").unwrap();
    assert_eq!(
        session.compile_fragment(2),
        compile_fragment_platform_impl(src, "balanced", false, 2, "", &platform)
    );
}
