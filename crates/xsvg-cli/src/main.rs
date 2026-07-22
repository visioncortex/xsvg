//! Headless xsvg compiler — a pure-Rust binary, no browser. Wires the native platform
//! seams (fonts via ttf-parser, shape flow via kurbo) to the shared `xsvg-core` compiler.
//!
//!   xsvg [--quality fast|balanced|highest] [--sourcemap] [-o OUT] INPUT
//!
//! INPUT is a path or `-` for stdin; OUT defaults to stdout.

mod fonts;
mod platform;

use fonts::FontDb;
use std::io::{Read, Write};
use std::path::Path;
use std::process::ExitCode;

/// Resolves cross-file `<use href="…">` links from disk, relative to the referencing
/// file's directory. The canonical key is the resolved absolute path (so the compiler's
/// cycle guard compares stable keys); on-demand reads are fine since native fs is sync.
struct DiskResolver;
impl xsvg_core::Resolver for DiskResolver {
    fn resolve(&self, base: &str, href: &str) -> Option<(String, String)> {
        let dir = Path::new(base).parent().unwrap_or_else(|| Path::new("."));
        let canon = std::fs::canonicalize(dir.join(href)).ok()?;
        let text = std::fs::read_to_string(&canon).ok()?;
        Some((canon.to_string_lossy().into_owned(), text))
    }
}

/// Compile an xsvg document to plain SVG. `font_dir` supplies the fonts used for text
/// measurement and `outline="true"` baking; with `None` (or an empty directory) text
/// falls back to default metrics and stays live `<text>` (no outline baking). `base` is
/// the source's path — the anchor for resolving relative `<use href>` cross-file links.
pub fn compile(
    source: &str,
    quality: &str,
    sourcemap: bool,
    font_dir: Option<&Path>,
    base: &str,
) -> Result<String, String> {
    let fonts = match font_dir {
        Some(d) => {
            FontDb::load_dir(d).map_err(|e| format!("loading fonts from {}: {e}", d.display()))?
        }
        None => FontDb::default(),
    };
    let p = platform::Native { fonts };
    xsvg_core::compile_linked_impl(source, quality, sourcemap, &p, &p, &p, &DiskResolver, base)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xsvg: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut quality = "balanced".to_string();
    let mut sourcemap = false;
    let mut font_dir: Option<String> = None;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" | "--output" => {
                output = Some(args.next().ok_or("-o needs a path")?);
            }
            "--quality" | "-q" => {
                quality = args.next().ok_or("--quality needs a value")?;
            }
            "--font-directory" | "--fonts" => {
                font_dir = Some(args.next().ok_or("--font-directory needs a path")?);
            }
            "--sourcemap" => sourcemap = true,
            "-h" | "--help" => {
                println!(
                    "usage: xsvg [--quality fast|balanced|highest] [--font-directory DIR] \
                     [--sourcemap] [-o OUT] INPUT\n\
                     INPUT is a path or - for stdin; OUT defaults to stdout.\n\
                     --font-directory loads .ttf/.otf fonts for text measurement and \
                     outline baking (without it, text uses default metrics and stays live)."
                );
                return Ok(());
            }
            _ if input.is_none() => input = Some(a),
            _ => return Err(format!("unexpected argument: {a}")),
        }
    }

    let input = input.ok_or("missing INPUT (path or - for stdin)")?;
    let (source, base) = if input == "-" {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| format!("reading stdin: {e}"))?;
        (s, String::new()) // stdin: relative <use href> resolves against the cwd
    } else {
        let s = std::fs::read_to_string(&input).map_err(|e| format!("reading {input}: {e}"))?;
        // canonical path anchors relative <use href> links (falls back to the raw arg)
        let base = std::fs::canonicalize(&input)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| input.clone());
        (s, base)
    };

    let svg = compile(
        &source,
        &quality,
        sourcemap,
        font_dir.as_deref().map(Path::new),
        &base,
    )?;

    match output {
        Some(path) => std::fs::write(&path, svg).map_err(|e| format!("writing {path}: {e}"))?,
        None => std::io::stdout()
            .write_all(svg.as_bytes())
            .map_err(|e| format!("writing stdout: {e}"))?,
    }
    Ok(())
}
