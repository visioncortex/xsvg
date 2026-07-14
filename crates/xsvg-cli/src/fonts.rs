//! Fonts loaded from a directory at runtime (`--font-directory`) rather than embedded, so
//! the binary carries no font bytes and each font keeps its license file beside it. Every
//! `.ttf`/`.otf` in the directory is classified into a role by its family/italic bits; a
//! run's `(family, weight, style)` then resolves to one role.

use std::path::Path;
use ttf_parser::{Face, Tag};

/// The loaded fonts, one slot per role.
#[derive(Default)]
pub struct FontDb {
    display: Option<Vec<u8>>,     // matched by family name (e.g. Anton)
    sans: Option<Vec<u8>>,        // upright sans (e.g. Arimo)
    sans_italic: Option<Vec<u8>>, // italic sans
}

impl FontDb {
    /// Load every `.ttf`/`.otf` in `dir`, routing each into a role by its family name and
    /// italic bit. Unreadable or unparseable files are skipped; the first file to claim a
    /// role wins.
    pub fn load_dir(dir: &Path) -> std::io::Result<Self> {
        let mut db = FontDb::default();
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext != "ttf" && ext != "otf" {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            // read the classification bits, then drop the borrow before moving `bytes`
            let (fam, italic) = match Face::parse(&bytes, 0) {
                Ok(face) => (family_name(&face).to_ascii_lowercase(), face.is_italic()),
                Err(_) => continue,
            };
            let slot = if fam.contains("anton") {
                &mut db.display
            } else if italic {
                &mut db.sans_italic
            } else {
                &mut db.sans
            };
            slot.get_or_insert(bytes);
        }
        Ok(db)
    }

    /// Font bytes for a run's `(family, style)`, with fallbacks: a named display font falls
    /// back to the sans, and italic falls back to the upright sans.
    pub fn resolve(&self, family: &str, style: &str) -> Option<&[u8]> {
        let fam = family.to_ascii_lowercase();
        let italic = style.eq_ignore_ascii_case("italic") || style.eq_ignore_ascii_case("oblique");
        let pick = if fam.contains("anton") {
            self.display.as_ref().or(self.sans.as_ref())
        } else if italic {
            self.sans_italic.as_ref().or(self.sans.as_ref())
        } else {
            self.sans.as_ref()
        };
        pick.map(Vec::as_slice)
    }
}

/// Parse `bytes` into a `Face`, dialing the `wght` axis (a no-op for static fonts).
pub fn face(bytes: &[u8], weight: f32) -> Option<Face<'_>> {
    let mut f = Face::parse(bytes, 0).ok()?;
    f.set_variation(Tag::from_bytes(b"wght"), weight);
    Some(f)
}

/// CSS `font-weight` → a numeric axis value.
pub fn parse_weight(weight: &str) -> f32 {
    match weight.trim().to_ascii_lowercase().as_str() {
        "" | "normal" => 400.0,
        "bold" | "bolder" => 700.0,
        "lighter" => 300.0,
        s => s.parse::<f32>().unwrap_or(400.0).clamp(1.0, 1000.0),
    }
}

/// Typographic family (name id 16) if present, else the legacy family (name id 1).
fn family_name(face: &Face) -> String {
    let mut fam = String::new();
    for name in face.names() {
        match name.name_id {
            16 => {
                if let Some(s) = name.to_string() {
                    return s;
                }
            }
            1 if fam.is_empty() => {
                if let Some(s) = name.to_string() {
                    fam = s;
                }
            }
            _ => {}
        }
    }
    fam
}
