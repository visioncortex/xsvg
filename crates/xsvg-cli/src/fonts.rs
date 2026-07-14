//! Bundled fonts + family resolution. The fonts are embedded (see
//! `assets/fonts/README.md`) so the CLI is self-contained and reproducible — no system
//! fonts, no network. A run's `(family, weight, style)` resolves to one of them.

use ttf_parser::{Face, Tag};

// Embedded font bytes (sources + licenses in assets/fonts/README.md).
static ANTON: &[u8] = include_bytes!("../../../assets/fonts/Anton-Regular.ttf");
static ARIMO: &[u8] = include_bytes!("../../../assets/fonts/Arimo[wght].ttf");
static ARIMO_ITALIC: &[u8] = include_bytes!("../../../assets/fonts/Arimo-Italic[wght].ttf");

/// A resolved font: bytes to parse plus the `wght`-axis value (ignored for the static
/// display font).
pub struct Resolved {
    pub bytes: &'static [u8],
    pub weight: f32,
    pub variable: bool,
}

/// Map a run's `(family, weight, style)` to a bundled font. Anton is matched by name —
/// it's baked to outlines in samples, so it must be the real font, glyph-for-glyph — and
/// everything else falls back to Arimo (Arial/Helvetica-metric-compatible), upright or
/// italic. The family attribute may be a CSS list (`'Helvetica Neue', Arial, sans-serif`);
/// a substring match is enough to route it.
pub fn resolve(family: &str, weight: &str, style: &str) -> Resolved {
    let fam = family.to_ascii_lowercase();
    let italic = style.eq_ignore_ascii_case("italic") || style.eq_ignore_ascii_case("oblique");
    let w = parse_weight(weight);
    if fam.contains("anton") {
        Resolved {
            bytes: ANTON,
            weight: w,
            variable: false,
        }
    } else if italic {
        Resolved {
            bytes: ARIMO_ITALIC,
            weight: w,
            variable: true,
        }
    } else {
        Resolved {
            bytes: ARIMO,
            weight: w,
            variable: true,
        }
    }
}

/// CSS `font-weight` → a numeric axis value.
fn parse_weight(weight: &str) -> f32 {
    match weight.trim().to_ascii_lowercase().as_str() {
        "" | "normal" => 400.0,
        "bold" | "bolder" => 700.0,
        "lighter" => 300.0,
        s => s.parse::<f32>().unwrap_or(400.0).clamp(1.0, 1000.0),
    }
}

/// Parse into a `Face`, dialing the `wght` axis for the variable fonts.
pub fn face(r: &Resolved) -> Option<Face<'static>> {
    let mut f = Face::parse(r.bytes, 0).ok()?;
    if r.variable {
        f.set_variation(Tag::from_bytes(b"wght"), r.weight);
    }
    Some(f)
}
