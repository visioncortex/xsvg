#!/usr/bin/env bash
# Take a headless screenshot of a rendered xsvg sample with Microsoft Edge.
#
# Runs Edge in headless mode with a virtual time budget so the page's JS + WASM
# compile pipeline finishes before the frame is captured. Used to visually verify
# the compiler's output while the dev server (`npm run dev`) is running.
#
# Usage:
#   .claude/screenshot.sh <url-or-sample> <out.png> [WxH] [budget-ms]
#
#   <url-or-sample>  full http(s):// URL, or a dataset sample name
#                    ("tbreak-and-glyph-scale" → http://localhost:5173/preview/?file=<name>.xsvg)
#   <out.png>        output path for the PNG
#   [WxH]            window size, default 1160x640
#   [budget-ms]      virtual time budget, default 6000
#
# Examples:
#   .claude/screenshot.sh tbreak-and-glyph-scale /tmp/shot.png
#   .claude/screenshot.sh textarea-ellipsis /tmp/e.png 1160x400
#   .claude/screenshot.sh http://localhost:5173/preview/?file=cards.xsvg /tmp/c.png
set -euo pipefail

EDGE="/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"

if [[ $# -lt 2 ]]; then
  echo "usage: $0 <url-or-sample> <out.png> [WxH] [budget-ms]" >&2
  exit 2
fi

target="$1"
out="$2"
size="${3:-1160x640}"
budget="${4:-6000}"

if [[ "$target" != http://* && "$target" != https://* ]]; then
  name="$target"
  [[ "$name" == *.xsvg ]] || name="$name.xsvg"
  url="http://localhost:5173/preview/?file=$name"
else
  url="$target"
fi

if [[ ! -x "$EDGE" ]]; then
  echo "error: Microsoft Edge not found at $EDGE" >&2
  exit 1
fi

"$EDGE" \
  --headless=new --disable-gpu --hide-scrollbars \
  --virtual-time-budget="$budget" \
  --window-size="${size/x/,}" \
  --screenshot="$out" \
  "$url" 2>/dev/null

echo "wrote $out ($url, ${size})"
