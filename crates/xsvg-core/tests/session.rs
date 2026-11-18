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
    fragment_range_impl, node_spans_impl, text_layout_impl, FittedMesh, FontMetrics, MeasureCache,
    Measurer, MeshCache, MeshFits, MeshKey, NodeKeys, Platform, Resolver, TextStyle,
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
        (1..=4).contains(&added),
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

// ---- the mesh cache (docs/Compile Session.md §5) ----

const XW: &str =
    r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">"#;
const MESH: &str = r##"<x:mesh id="m" points="0,0 100,0 100,50 0,50"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/></x:mesh>"##;

fn compile_on<P: xsvg_core::Host>(platform: &P, src: &str, quality: &str) -> String {
    xsvg_core::CompileSession::new(platform, src, quality, false, "")
        .unwrap()
        .compile()
        .unwrap()
}

#[test]
fn the_mesh_cache_changes_no_output() {
    // one cache carried across the corpus, every quality, cold and warm, whole
    // documents and fragments, compact and display-resolution textures
    let cache = MeshCache::new();
    let mut meshes = 0;
    for (base, src) in samples() {
        if !src.contains("x:mesh") && !src.contains("meshgradient") {
            continue;
        }
        meshes += 1;
        let (tops, _) = offsets(&src);
        for quality in ["fast", "balanced", "highest", "raster"] {
            let plain = Platform::new(Mono, BoxShaper, BoxOutliner).with_resolver(Dataset);
            let cached = Platform::new(Mono, BoxShaper, BoxOutliner)
                .with_resolver(Dataset)
                .with_mesh_cache(&cache);
            let mode = format!("{base} {quality}");
            let want = compile_on(&plain, &src, quality);
            assert_eq!(compile_on(&cached, &src, quality), want, "{mode} cold");
            assert_eq!(compile_on(&cached, &src, quality), want, "{mode} warm");
            for &at in tops
                .iter()
                .filter(|_| quality == "fast" || quality == "balanced")
            {
                assert_eq!(
                    compile_fragment_platform_impl(&src, quality, true, at, &base, &cached),
                    compile_fragment_platform_impl(&src, quality, true, at, &base, &plain),
                    "{mode} fragment {at}"
                );
            }
            let dense = |p: &dyn Fn() -> String| p();
            let at_density = |cached_run: bool| {
                dense(&|| {
                    let session = if cached_run {
                        cached.session(&src, quality, false, &base).map(|s| {
                            s.with_display_resolution(Some(2.0))
                                .unwrap()
                                .compile()
                                .unwrap()
                        })
                    } else {
                        plain.session(&src, quality, false, &base).map(|s| {
                            s.with_display_resolution(Some(2.0))
                                .unwrap()
                                .compile()
                                .unwrap()
                        })
                    };
                    session.unwrap()
                })
            };
            if quality == "balanced" {
                assert_eq!(
                    at_density(true),
                    at_density(false),
                    "{mode} at display resolution"
                );
            }
        }
    }
    assert!(meshes >= 4, "the corpus has mesh samples: {meshes}");
    assert!(
        cache.hits() > cache.misses(),
        "{} hits, {} misses",
        cache.hits(),
        cache.misses()
    );
}

#[test]
fn a_second_flush_fits_nothing_the_first_one_did() {
    let cache = MeshCache::new();
    // a platform per flush, as an embedder builds them; the cache outlives each
    let flush = |src: &str, quality: &str| {
        let platform = Platform::new(Mono, BoxShaper, BoxOutliner).with_mesh_cache(&cache);
        compile_on(&platform, src, quality)
    };
    let doc = |shape: &str, mesh: &str| format!("{XW}<defs>{mesh}</defs>{shape}</svg>");
    let shape = r##"<path d="M10,10 L90,10 L50,45 Z" fill="url(#m)"/>"##;

    // flush 1: the mesh is fitted once, though it is written twice — where it
    // is defined and in the shape it fills
    let first = flush(&doc(shape, MESH), "balanced");
    assert_eq!((cache.misses(), cache.hits()), (1, 1));
    assert_eq!(first.matches("<image").count(), 2);

    // flush 2 over the same text: the same output, and no fit
    assert_eq!(flush(&doc(shape, MESH), "balanced"), first);
    assert_eq!((cache.misses(), cache.hits()), (1, 3));

    // the shape moves, reshapes, gains a stroke and neighbours: the mesh did
    // not change, so nothing is fitted
    for edited in [
        r##"<path d="M10,10 L90,10 L50,45 Z" transform="translate(40 7) rotate(12)" fill="url(#m)"/>"##,
        r##"<path d="M0,0 C40,-20 80,60 20,45 Z" fill="url(#m)" stroke="#222"/>"##,
        r##"<rect width="9" height="9" fill="url(#m)"/><circle r="30" fill="url(#m)" stroke="url(#m)"/><x:boolean fill="url(#m)"><rect width="50" height="20"/><rect x="20" width="50" height="40"/></x:boolean>"##,
    ] {
        flush(&doc(edited, MESH), "balanced");
    }
    assert_eq!(cache.misses(), 1, "{} hits", cache.hits());
    assert_eq!(cache.len(), 1);

    // the mesh changes — one colour, one coordinate: each is another mesh
    flush(&doc(shape, &MESH.replace("#0f0", "#0e0")), "balanced");
    assert_eq!(cache.misses(), 2);
    flush(&doc(shape, &MESH.replace("100,50", "100,51")), "balanced");
    assert_eq!(cache.misses(), 3);
    // and back again is a hit: nothing was forgotten
    flush(&doc(shape, MESH), "balanced");
    assert_eq!(cache.misses(), 3);

    // a fit belongs to its quality; `raster` fits as `highest` does
    flush(&doc(shape, MESH), "fast");
    assert_eq!(cache.misses(), 4);
    flush(&doc(shape, MESH), "highest");
    assert_eq!(cache.misses(), 5);
    flush(&doc(shape, MESH), "raster");
    assert_eq!(cache.misses(), 5);
    assert_eq!(cache.len(), 5);
}

#[test]
fn what_places_a_mesh_decides_whether_its_fit_is_shared() {
    let cache = MeshCache::new();
    let platform = Platform::new(Mono, BoxShaper, BoxOutliner).with_mesh_cache(&cache);
    // the same mesh written twice under different ids is one mesh
    let twice = format!(
        "{XW}{MESH}{}<g transform=\"scale(3)\">{}</g></svg>",
        MESH.replace("id=\"m\"", "id=\"n\""),
        MESH.replace("id=\"m\"", "id=\"o\"")
    );
    compile_on(&platform, &twice, "balanced");
    assert_eq!((cache.misses(), cache.hits()), (1, 2));

    // a paint's own transform, and bounding-box units, move the mesh itself:
    // every distinct placement is fitted, every repeated one is not
    cache.clear();
    let before = cache.misses();
    let unit = r##"<x:mesh id="u" units="objectBoundingBox" points="0,0 1,0 1,1 0,1"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/></x:mesh>"##;
    let boxed = format!(
        r##"{XW}<defs>{unit}</defs><rect width="40" height="20" fill="url(#u)"/><rect x="100" width="40" height="20" fill="url(#u)"/><rect width="40" height="20" stroke="url(#u)" fill="url(#u)"/><rect transform="translate(500)" width="40" height="20" fill="url(#u)"/></svg>"##
    );
    compile_on(&platform, &boxed, "balanced");
    // the definition (unit square), the box at the origin (×4 uses), the box at 100
    assert_eq!(cache.misses() - before, 3);
    assert_eq!(cache.len(), 3);

    // a <meshgradient> is fitted as the mesh it tessellates to, and cached alike
    cache.clear();
    let before = cache.misses();
    let mg = r##"<meshgradient id="g" x="0" y="0" gradientUnits="userSpaceOnUse"><meshrow><meshpatch><stop path="c 30,-10 70,10 100,0" stop-color="#f00"/><stop path="l 0,50" stop-color="#0f0"/><stop path="l -100,0" stop-color="#00f"/><stop path="l 0,-50" stop-color="#fff"/></meshpatch></meshrow></meshgradient>"##;
    let filled = format!(
        r##"{XW}<defs>{mg}</defs><rect width="90" height="40" fill="url(#g)"/><circle cx="40" cy="20" r="15" fill="url(#g)"/></svg>"##
    );
    let cold = compile_on(&platform, &filled, "highest");
    assert_eq!(cache.misses() - before, 1);
    assert_eq!(compile_on(&platform, &filled, "highest"), cold);
    assert_eq!(cache.misses() - before, 1);
}

#[test]
fn an_embedder_s_own_store_is_asked_and_offered() {
    // any store will do: this one keeps fits by the key's text, and a log
    #[derive(Default)]
    struct Shelf {
        fits: std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<FittedMesh>>>,
        log: std::sync::Mutex<Vec<String>>,
    }
    impl MeshFits for Shelf {
        fn fitted(&self, key: &MeshKey) -> Option<std::sync::Arc<FittedMesh>> {
            let hit = self.fits.lock().unwrap().get(&key.to_string()).cloned();
            let what = if hit.is_some() { "hit" } else { "miss" };
            self.log.lock().unwrap().push(format!("{what} {key}"));
            hit
        }
        fn keep(&self, key: MeshKey, fit: &std::sync::Arc<FittedMesh>) {
            assert!(fit.weight() > 0);
            self.log.lock().unwrap().push(format!("keep {key}"));
            self.fits
                .lock()
                .unwrap()
                .insert(key.to_string(), fit.clone());
        }
    }
    let src = format!("{XW}{MESH}{}</svg>", MESH.replace("id=\"m\"", "id=\"n\""));
    let plain = compile_on(
        &Platform::new(Mono, BoxShaper, BoxOutliner),
        &src,
        "balanced",
    );
    // owned by the platform this time
    let platform = Platform::new(Mono, BoxShaper, BoxOutliner).with_mesh_cache(Shelf::default());
    assert_eq!(compile_on(&platform, &src, "balanced"), plain);
    assert_eq!(compile_on(&platform, &src, "balanced"), plain);
    // the builder keeps the store wherever it is called in the chain
    let chained = Platform::new(Mono, BoxShaper, BoxOutliner)
        .with_mesh_cache(Shelf::default())
        .with_resolver(Dataset)
        .with_keys(SpanKeys(false))
        .with_measure_cache();
    assert_eq!(compile_on(&chained, &src, "balanced"), plain);
    assert_eq!(compile_on(&chained, &src, "balanced"), plain);
}

#[test]
fn a_mesh_with_nothing_to_fit_is_never_kept() {
    let cache = MeshCache::new();
    let platform = Platform::new(Mono, BoxShaper, BoxOutliner).with_mesh_cache(&cache);
    let src = format!(
        r##"{XW}<x:mesh points="0,0 10,0 20,0 30,0"><x:face v="0 1 2 3" fill="#f00"/></x:mesh><x:mesh points="0,0 1,1"/></svg>"##
    );
    let out = compile_on(&platform, &src, "balanced");
    assert!(
        out.contains("degenerate extent") && out.contains("no usable faces"),
        "{out}"
    );
    assert!(cache.is_empty());
    assert_eq!(compile_on(&platform, &src, "balanced"), out);
}

#[test]
fn cached_and_fresh_agree_on_arbitrary_meshes() {
    // random meshes — grids with jittered vertices, cracks, triangles, alpha,
    // degenerate faces — compiled fresh, cold, warm, and after the cache
    // started over: always the same bytes
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let cache = MeshCache::new();
    let plain = Platform::new(Mono, BoxShaper, BoxOutliner);
    let cached = Platform::new(Mono, BoxShaper, BoxOutliner).with_mesh_cache(&cache);
    let mut textured = 0;
    for round in 0..60 {
        let (cols, rows) = (1 + next() % 4, 1 + next() % 3);
        let stretch = [1.0, 1.0, 0.05, 400.0][(next() % 4) as usize];
        let mut points = String::new();
        for j in 0..=rows {
            for i in 0..=cols {
                let jitter = |n: u64| (n % 7) as f64 - 3.0;
                points.push_str(&format!(
                    "{},{} ",
                    (i as f64 * 20.0 + jitter(next())) * stretch,
                    j as f64 * 20.0 + jitter(next())
                ));
            }
        }
        let colour = |n: u64| {
            let palette = [
                "#f00", "#0f0", "#00f", "#fff", "#000", "#fa0", "#0af8", "#f0f4",
            ];
            palette[(n % palette.len() as u64) as usize]
        };
        let mut faces = String::new();
        for j in 0..rows {
            for i in 0..cols {
                let v = |di: u64, dj: u64| (j + dj) * (cols + 1) + i + di;
                let quad = [v(0, 0), v(1, 0), v(1, 1), v(0, 1)];
                match next() % 6 {
                    0 => faces.push_str(&format!(
                        r##"<x:face v="{} {} {}" fill="{} {} {}"/>"##,
                        quad[0],
                        quad[1],
                        quad[2],
                        colour(next()),
                        colour(next()),
                        colour(next())
                    )),
                    1 => faces.push_str(&format!(
                        r##"<x:face v="{} {} {} {}" fill="{}"/>"##,
                        quad[0],
                        quad[1],
                        quad[2],
                        quad[3],
                        colour(next())
                    )),
                    2 => faces.push_str(&format!(
                        r##"<x:face v="{} {} {} {}" fill="#111 #222 #333 #444"/>"##,
                        quad[0], quad[0], quad[1], quad[1]
                    )),
                    _ => faces.push_str(&format!(
                        r##"<x:face v="{} {} {} {}" fill="{} {} {} {}"/>"##,
                        quad[0],
                        quad[1],
                        quad[2],
                        quad[3],
                        colour(next()),
                        colour(next()),
                        colour(next()),
                        colour(next())
                    )),
                }
            }
        }
        let quality = ["fast", "balanced", "fast", "balanced", "highest"][(next() % 5) as usize];
        let src = format!(
            r##"{XW}<defs><x:mesh id="m" points="{points}">{faces}</x:mesh></defs><rect x="5" y="5" width="60" height="40" fill="url(#m)" stroke="url(#m)" stroke-width="3"/><x:mesh points="{points}" transform="scale(2)">{faces}</x:mesh></svg>"##
        );
        let want = compile_on(&plain, &src, quality);
        assert_eq!(
            compile_on(&cached, &src, quality),
            want,
            "round {round} cold\n{src}"
        );
        assert_eq!(
            compile_on(&cached, &src, quality),
            want,
            "round {round} warm\n{src}"
        );
        if round % 20 == 19 {
            cache.clear();
            assert_eq!(
                compile_on(&cached, &src, quality),
                want,
                "round {round} after a clear"
            );
        }
        textured += want.matches("<image").count();
    }
    assert!(textured > 100, "the sweep exercised textures: {textured}");
    assert!(cache.hits() > cache.misses() * 3);
}

// ---- a mesh on its own: palette swatches (§8.2) ----

/// Every `<image …/>` of a compile, without the clip id it names.
fn images(out: &str) -> Vec<String> {
    out.match_indices("<image")
        .map(|(at, _)| {
            let image = &out[at..at + out[at..].find("/>").unwrap()];
            let Some(clip) = image.find(" clip-path=\"") else {
                return image.to_string();
            };
            let close = clip + 12 + image[clip + 12..].find('"').unwrap() + 1;
            format!("{}{}", &image[..clip], &image[close..])
        })
        .collect()
}

fn ids(svg: &str) -> Vec<String> {
    roxmltree::Document::parse(svg)
        .unwrap()
        .descendants()
        .filter_map(|n| n.attribute("id").map(str::to_string))
        .collect()
}

#[test]
fn a_document_s_meshes_are_listed_and_each_compiles_on_its_own() {
    let mg = r##"<meshgradient id="g" x="10" y="20" gradientUnits="userSpaceOnUse"><meshrow><meshpatch><stop path="l 60,0" stop-color="#f00"/><stop path="l 0,30" stop-color="#0f0"/><stop path="l -60,0" stop-color="#00f"/><stop path="l 0,-30" stop-color="#fff"/></meshpatch></meshrow></meshgradient>"##;
    let cracked = r##"<x:mesh id="c" points="0,0 50,0 100,0 0,50 50,50 100,50"><x:face v="0 1 4 3" fill="#f00 #0f0 #00f #fff"/><x:face v="1 2 5 4" fill="#000 #f0f #ff0 #000"/></x:mesh>"##;
    let src = format!(
        r##"{XW}<defs>{MESH}{mg}</defs><rect width="9" height="9" fill="url(#m)"/><g>{cracked}</g><x:doc>{MESH}</x:doc></svg>"##
    );
    let cache = MeshCache::new();
    let platform = Platform::new(Mono, BoxShaper, BoxOutliner).with_mesh_cache(&cache);
    let session = platform.session(&src, "balanced", false, "").unwrap();

    // every definition the compiler sees, wherever it stands, in order
    let listed = session.mesh_definitions();
    let starts: Vec<usize> = listed.iter().map(|r| r.0).collect();
    assert_eq!(
        starts,
        vec![
            src.find("<x:mesh").unwrap(),
            src.find("<meshgradient").unwrap(),
            src.find(r#"<x:mesh id="c""#).unwrap(),
        ],
        "not the one inside <x:doc>"
    );
    for &(start, end) in &listed {
        assert!(src[start..end].starts_with('<') && src[start..end].ends_with('>'));
    }

    let whole = session.compile().unwrap();
    let fitted = cache.misses();
    let swatches: Vec<_> = starts
        .iter()
        .map(|&at| session.mesh_swatch(at).unwrap())
        .collect();
    // the document's compile already fitted the two it draws; the gradient no
    // shape names is fitted here, once
    assert_eq!(cache.misses(), fitted + 1);
    let again: Vec<_> = starts
        .iter()
        .map(|&at| session.mesh_swatch(at).unwrap())
        .collect();
    assert_eq!(again, swatches);
    assert_eq!(
        cache.misses(),
        fitted + 1,
        "a swatch seen before costs no fit"
    );

    for swatch in &swatches {
        let doc = roxmltree::Document::parse(&swatch.svg).unwrap();
        let root = doc.root_element();
        assert_eq!(
            root.tag_name().namespace(),
            Some("http://www.w3.org/2000/svg")
        );
        let view: Vec<f64> = root
            .attribute("viewBox")
            .unwrap()
            .split(' ')
            .map(|v| v.parse().unwrap())
            .collect();
        assert_eq!(view, swatch.bounds, "{}", swatch.svg);
        // nothing of the document comes along
        assert!(
            !swatch.svg.contains("xsvg") && !swatch.svg.contains("x:"),
            "{}",
            swatch.svg
        );
        // every clip it names is its own, and named after the mesh
        let own = ids(&swatch.svg);
        assert!(own.iter().all(|i| i.contains(&swatch.key.to_string())));
        for n in doc.descendants() {
            if let Some(clip) = n.attribute("clip-path") {
                let id = clip.trim_start_matches("url(#").trim_end_matches(')');
                assert!(own.iter().any(|i| i == id), "{clip}");
            }
        }
    }
    assert_eq!(swatches[0].bounds, [0.0, 0.0, 100.0, 50.0]);
    assert_eq!(swatches[1].bounds, [10.0, 20.0, 60.0, 30.0]);
    // a swatch is a fill of its box: a mesh without a crack is one image,
    // unclipped, and has no id at all
    for whole in [&swatches[0], &swatches[1]] {
        assert_eq!(images(&whole.svg).len(), 1, "{}", whole.svg);
        assert!(
            !whole.svg.contains("clip") && ids(&whole.svg).is_empty(),
            "{}",
            whole.svg
        );
        covers(whole);
    }
    // a crack keeps its edge: the far side is clipped over the near one
    assert_eq!(
        images(&swatches[2].svg).len(),
        2,
        "one texture per crack region"
    );
    assert_eq!(swatches[2].svg.matches("<clipPath").count(), 1);
    assert_eq!(swatches[2].svg.matches(" clip-path=").count(), 1);
    assert!(
        swatches[2].svg.find("<rect").unwrap() < swatches[2].svg.find("<image").unwrap(),
        "half a mesh's image stops short of the box, so a ground lies under it: {}",
        swatches[2].svg
    );

    // the picture is the one the document draws for that mesh (the gradient
    // is drawn nowhere in it)
    let drawn = images(&whole);
    for swatch in [&swatches[0], &swatches[2]] {
        for image in images(&swatch.svg) {
            assert!(drawn.contains(&image), "{image}");
        }
    }
    // swatches of different meshes can share a DOM: no id is used twice
    let mut all: Vec<String> = swatches.iter().flat_map(|s| ids(&s.svg)).collect();
    let count = all.len();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), count);
}

/// The swatch's first, unclipped paint reaches every edge of its box.
fn covers(swatch: &xsvg_core::MeshSwatch) {
    let doc = roxmltree::Document::parse(&swatch.svg).unwrap();
    let ground = doc
        .root_element()
        .children()
        .find(|n| n.is_element())
        .unwrap();
    assert!(
        matches!(ground.tag_name().name(), "image" | "rect")
            && ground.attribute("clip-path").is_none(),
        "{}",
        swatch.svg
    );
    let at = |name: &str| ground.attribute(name).unwrap().parse::<f64>().unwrap();
    let [x, y, w, h] = swatch.bounds;
    assert!(
        at("x") <= x
            && at("y") <= y
            && at("x") + at("width") >= x + w
            && at("y") + at("height") >= y + h,
        "{}",
        swatch.svg
    );
}

#[test]
fn a_swatch_fills_its_box_whatever_shape_the_mesh_is() {
    let swatch = |mesh: &str| {
        let src = format!("{XW}{mesh}</svg>");
        let platform = Platform::new(Mono, BoxShaper, BoxOutliner);
        let session = platform.session(&src, "highest", false, "").unwrap();
        session.mesh_swatch(src.find("<x:mesh").unwrap()).unwrap()
    };
    // a triangle, an L of three squares, a quad with a notch: each
    // leaves part of its bounding box empty
    for mesh in [
        r##"<x:mesh points="0,0 100,0 0,80"><x:face v="0 1 2" fill="#c33 #3c3 #33c"/></x:mesh>"##,
        r##"<x:mesh points="0,0 32,0 0,32 32,32 0,64 32,64 64,32 64,64"><x:face v="0 1 3 2" fill="#a49e9f #9d9799 #856976 #886f7b"/><x:face v="2 3 5 4" fill="#886f7b #856976 #7b5f6d #7a606d"/><x:face v="3 6 7 5" fill="#856976 #8e7e85 #775d6a #7b5f6d"/></x:mesh>"##,
        r##"<x:mesh points="0,0 60,0 60,20 30,20 30,40 0,40"><x:face v="0 1 2 3" fill="#fc0 #f60 #f06 #c0f"/><x:face v="0 3 4 5" fill="#fc0 #c0f #60f #06f"/></x:mesh>"##,
    ] {
        let s = swatch(mesh);
        assert!(!s.svg.contains("clip"), "{}", s.svg);
        assert_eq!(images(&s.svg).len(), 1, "{}", s.svg);
        covers(&s);
    }
    // one colour throughout: a rectangle of it
    let flat =
        swatch(r##"<x:mesh points="5,5 45,5 5,35"><x:face v="0 1 2" fill="#336699"/></x:mesh>"##);
    assert_eq!(
        flat.svg,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="5 5 40 30"><rect x="5" y="5" width="40" height="30" fill="#336699"/></svg>"##
    );
    // the document still draws the mesh as the shape it is
    let src = format!(
        "{XW}{}</svg>",
        r##"<x:mesh points="0,0 100,0 0,80"><x:face v="0 1 2" fill="#c33 #3c3 #33c"/></x:mesh>"##
    );
    let out = Platform::new(Mono, BoxShaper, BoxOutliner)
        .session(&src, "highest", false, "")
        .unwrap()
        .compile()
        .unwrap();
    assert!(
        out.contains("<clipPath") && out.contains(" clip-path="),
        "{out}"
    );
}

#[test]
fn a_swatch_s_key_changes_exactly_when_the_mesh_does() {
    let platform = Platform::new(Mono, BoxShaper, BoxOutliner);
    let swatch = |src: &str, quality: &str| {
        let session = platform.session(src, quality, false, "").unwrap();
        let at = session.mesh_definitions()[0].0;
        session.mesh_swatch(at).unwrap()
    };
    let base = swatch(&format!("{XW}{MESH}</svg>"), "balanced");
    // the same mesh somewhere else in another document, under another id, used
    // or not: the same swatch, byte for byte
    for other in [
        format!(
            "{XW}<rect width=\"900\" height=\"9\"/><defs><g transform=\"scale(7)\">{MESH}</g></defs></svg>"
        ),
        format!(
            "{XW}<!-- shifted -->{}<circle r=\"4\" fill=\"url(#renamed)\"/></svg>",
            MESH.replace("id=\"m\"", "id=\"renamed\"")
        ),
        format!(
            "{XW}{}</svg>",
            MESH.replace("points=\"0,0 100,0", "points=\"  0,0   100.0,0")
        ),
    ] {
        assert_eq!(swatch(&other, "balanced"), base, "{other}");
    }
    // a colour, a coordinate, a transform of its own, the quality: another key
    let mut keys = vec![base.key];
    for (changed, quality) in [
        (MESH.replace("#0f0", "#0e0"), "balanced"),
        (MESH.replace("100,50", "100,51"), "balanced"),
        (
            MESH.replace("<x:mesh ", "<x:mesh transform=\"scale(2)\" "),
            "balanced",
        ),
        (MESH.to_string(), "fast"),
    ] {
        let s = swatch(&format!("{XW}{changed}</svg>"), quality);
        assert!(!keys.contains(&s.key), "{changed} {quality}");
        assert_ne!(s.svg, base.svg);
        keys.push(s.key);
    }
    // the mesh's own transform is part of the paint it is
    let scaled = swatch(
        &format!(
            "{XW}{}</svg>",
            MESH.replace("<x:mesh ", "<x:mesh transform=\"scale(2)\" ")
        ),
        "balanced",
    );
    assert_eq!(scaled.bounds, [0.0, 0.0, 200.0, 100.0]);
    // bounding-box units have no box here: the unit square as written
    let unit = swatch(
        &format!(
            r##"{XW}<x:mesh units="objectBoundingBox" points="0,0 1,0 1,1 0,1"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/></x:mesh></svg>"##
        ),
        "balanced",
    );
    assert_eq!(unit.bounds, [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(images(&unit.svg).len(), 1);
}

#[test]
fn a_swatch_is_refused_where_there_is_no_mesh_to_show() {
    let src = format!(
        r##"{XW}<rect width="9" height="9"/><x:mesh points="0,0 1,1"/><x:mesh points="0,0 10,0 20,0 30,0"><x:face v="0 1 2 3" fill="#f00"/></x:mesh><meshgradient id="bad"><meshrow/></meshgradient></svg>"##
    );
    let platform = Platform::new(Mono, BoxShaper, BoxOutliner);
    let session = platform.session(&src, "balanced", false, "").unwrap();
    let err = |at: usize| session.mesh_swatch(at).unwrap_err();
    assert!(err(src.find("<rect").unwrap()).contains("not a mesh"));
    assert!(err(3).contains("no element starts"));
    let listed = session.mesh_definitions();
    assert_eq!(listed.len(), 3);
    assert!(err(listed[0].0).contains("nothing to draw"));
    assert!(err(listed[1].0).contains("no extent"));
    assert!(err(listed[2].0).contains("nothing to draw"));
}
