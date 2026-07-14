# Bundled fonts

Embedded into the `xsvg` CLI (`crates/xsvg-cli`) so the headless compiler is
self-contained and reproducible — no system fonts, no network. Used for text
measurement (line wrapping) and for baking `outline="true"` glyphs to `<path>`.

| File | Family | Role | License |
|---|---|---|---|
| `Anton-Regular.ttf` | Anton | Display; matched by name (`-x-google-Anton`). Baked to outlines in samples, so it must be the *real* font to match the browser. | OFL 1.1 |
| `Arimo[wght].ttf` | Arimo (variable) | Sans fallback for `Helvetica Neue` / `Arial` / `sans-serif`; metric-compatible with Arial. Weight via the `wght` axis. | OFL 1.1 |
| `Arimo-Italic[wght].ttf` | Arimo Italic (variable) | Italic sans fallback. | OFL 1.1 |

Both are licensed under the **SIL Open Font License 1.1**, whose terms require the
license (with each font's copyright notice) to travel with the font. The full texts
are included here:

- `Anton-OFL.txt` — © The Anton Project Authors
- `Arimo-OFL.txt` — © The Arimo Project Authors

Sources: `github.com/google/fonts` (`ofl/anton`, `ofl/arimo`).
