#!/usr/bin/env bash
# Build the Vite-friendly woff2 decoder (ESM .mjs + standalone .wasm) from source
# and install it into web/src/vendor/woff2/. Requires only Docker — the pinned
# emscripten image supplies emcc; nothing is installed on the host.
#
#   ./woff2/build.sh   (or: npm run woff2:build)
#
# Output is committed to the repo, so this only needs re-running to bump the
# woff2/brotli/emscripten versions or change the emcc flags.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
image="xsvg-woff2-builder"
out="$here/../web/src/vendor/woff2"

echo ">> [1/3] building emscripten image (google/woff2 + brotli static libs)…"
docker build --platform=linux/amd64 -t "$image" "$here"

echo ">> [2/3] linking decompress.mjs + decompress.wasm…"
# On Docker Desktop (macOS) bind-mount writes are mapped to the host user, so we
# run as root. On Linux, add `-u "$(id -u):$(id -g)"` to avoid root-owned output.
docker run --rm --platform=linux/amd64 \
  -v "$here":/src/wawoff2 \
  "$image" \
  make -C /src/wawoff2 build

echo ">> [3/3] installing into web/src/vendor/woff2/"
mkdir -p "$out"
cp "$here/build/decompress.mjs"  "$out/decompress.mjs"
cp "$here/build/decompress.wasm" "$out/decompress.wasm"

echo ">> done:"
ls -la "$out"/decompress.*
