//! Headless xsvg compiler — a pure-Rust binary, no browser. Wires the native platform
//! seams (fonts via ttf-parser, shape flow via kurbo) to the shared `xsvg-compile` core.
//!
//!   xsvg [--quality fast|balanced|highest] [--sourcemap] [-o OUT] INPUT
//!
//! INPUT is a path or `-` for stdin; OUT defaults to stdout.

mod fonts;
mod platform;

use std::io::{Read, Write};
use std::process::ExitCode;

/// Compile an xsvg document to plain SVG with the bundled native fonts. Exposed so the
/// comparison test harness can call it in-process.
pub fn compile(source: &str, quality: &str, sourcemap: bool) -> Result<String, String> {
    let p = platform::Native;
    xsvg_compile::compile_impl(source, quality, sourcemap, &p, &p, &p)
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

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" | "--output" => {
                output = Some(args.next().ok_or("-o needs a path")?);
            }
            "--quality" | "-q" => {
                quality = args.next().ok_or("--quality needs a value")?;
            }
            "--sourcemap" => sourcemap = true,
            "-h" | "--help" => {
                println!(
                    "usage: xsvg [--quality fast|balanced|highest] [--sourcemap] [-o OUT] INPUT\n\
                     INPUT is a path or - for stdin; OUT defaults to stdout."
                );
                return Ok(());
            }
            _ if input.is_none() => input = Some(a),
            _ => return Err(format!("unexpected argument: {a}")),
        }
    }

    let input = input.ok_or("missing INPUT (path or - for stdin)")?;
    let source = if input == "-" {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| format!("reading stdin: {e}"))?;
        s
    } else {
        std::fs::read_to_string(&input).map_err(|e| format!("reading {input}: {e}"))?
    };

    let svg = compile(&source, &quality, sourcemap)?;

    match output {
        Some(path) => std::fs::write(&path, svg).map_err(|e| format!("writing {path}: {e}"))?,
        None => std::io::stdout()
            .write_all(svg.as_bytes())
            .map_err(|e| format!("writing stdout: {e}"))?,
    }
    Ok(())
}
