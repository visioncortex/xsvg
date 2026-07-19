#!/usr/bin/env bash
#
# publish.sh — release xsvg in three parts:
#
#   1. rust     cargo-publish every publishable workspace crate, in dependency order
#   2. npm      build + npm-publish the two @visioncortex packages
#   3. release  attach the raw wasm binary to a GitHub release (via gh) — for
#               non-npm consumers; the npm packages already bundle the same wasm
#
# Which Rust crates ship is read from the Cargo manifests themselves: a crate with
# `publish = false` is skipped. All four (xsvg-gradient, xsvg-core, xsvg-cli, xsvg-wasm)
# are publishable and go to crates.io in dependency order; mark one `publish = false`
# to hold it back.
#
# Usage:
#   scripts/publish.sh                 # all three phases
#   scripts/publish.sh --npm           # just npm
#   scripts/publish.sh --rust --release
#   scripts/publish.sh --tag v0.1.0    # override the GitHub release tag
#   scripts/publish.sh --dry-run       # print every command, run nothing
#
# Flags: --rust  --npm  --release   (pick any subset; default = all)
#        --tag <tag>                 GitHub release tag (default: v<xsvg-viewer version>)
#        --dry-run                   show the plan without executing
#        --allow-dirty               pass --allow-dirty to cargo publish
#        -h | --help

set -euo pipefail

# ── locate the repo root (script lives in <root>/scripts) ───────────────────────
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# ── pretty logging ──────────────────────────────────────────────────────────────
if [[ -t 1 ]]; then
  BOLD=$'\033[1m'; DIM=$'\033[2m'; RED=$'\033[31m'; GRN=$'\033[32m'; YEL=$'\033[33m'; BLU=$'\033[34m'; RST=$'\033[0m'
else
  BOLD=''; DIM=''; RED=''; GRN=''; YEL=''; BLU=''; RST=''
fi
step() { printf '\n%s▶ %s%s\n' "$BOLD$BLU" "$*" "$RST"; }
info() { printf '%s  %s%s\n' "$DIM" "$*" "$RST"; }
ok()   { printf '%s✓ %s%s\n' "$GRN" "$*" "$RST"; }
warn() { printf '%s! %s%s\n' "$YEL" "$*" "$RST" >&2; }
die()  { printf '%s✗ %s%s\n' "$RED" "$*" "$RST" >&2; exit 1; }

# ── run a command (or just echo it in --dry-run) ────────────────────────────────
DRY_RUN=0
run() {
  if (( DRY_RUN )); then
    printf '%s$ %s%s\n' "$DIM" "$*" "$RST"
  else
    printf '%s$ %s%s\n' "$DIM" "$*" "$RST"
    "$@"
  fi
}

# ── argument parsing ────────────────────────────────────────────────────────────
DO_RUST=0; DO_NPM=0; DO_RELEASE=0; ANY=0
TAG=""; ALLOW_DIRTY=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --rust)        DO_RUST=1; ANY=1 ;;
    --npm)         DO_NPM=1; ANY=1 ;;
    --release)     DO_RELEASE=1; ANY=1 ;;
    --all)         DO_RUST=1; DO_NPM=1; DO_RELEASE=1; ANY=1 ;;
    --tag)         TAG="${2:-}"; [[ -n "$TAG" ]] || die "--tag needs a value"; shift ;;
    --tag=*)       TAG="${1#*=}" ;;
    --dry-run)     DRY_RUN=1 ;;
    --allow-dirty) ALLOW_DIRTY=1 ;;
    -h|--help)     sed -n '3,24p' "$0" | sed 's/^#\{0,1\} \{0,1\}//'; exit 0 ;;
    *)             die "unknown argument: $1 (try --help)" ;;
  esac
  shift
done
if (( ! ANY )); then DO_RUST=1; DO_NPM=1; DO_RELEASE=1; fi

# ── tool preflight ──────────────────────────────────────────────────────────────
need() { command -v "$1" >/dev/null 2>&1 || die "missing required tool: $1"; }
need node
(( DO_RUST ))                 && need cargo
(( DO_NPM || DO_RELEASE ))    && { need npm; need wasm-pack; }
(( DO_RELEASE ))              && need gh

VIEWER_VERSION="$(node -p "require('./packages/xsvg-viewer/package.json').version")"
: "${TAG:=v${VIEWER_VERSION}}"

printf '%srelease plan%s  rust=%d  npm=%d  release=%d  tag=%s  dry-run=%d\n' \
  "$BOLD" "$RST" "$DO_RUST" "$DO_NPM" "$DO_RELEASE" "$TAG" "$DRY_RUN"

# ════════════════════════════════════════════════════════════════════════════════
# 1. RUST — publish publishable workspace crates to crates.io, in dependency order
# ════════════════════════════════════════════════════════════════════════════════
publish_rust() {
  step "Rust — cargo publish"

  # Ask cargo which workspace crates are publishable, and order them so every
  # crate ships after the workspace crates it depends on.
  local crates
  crates="$(cargo metadata --no-deps --format-version 1 | node -e '
    const m = JSON.parse(require("fs").readFileSync(0, "utf8"));
    const members = new Set(m.packages.map(p => p.name));
    const byName  = Object.fromEntries(m.packages.map(p => [p.name, p]));
    // publish === [] means publish = false; null or a registry list means publishable.
    const publishable = p => !(Array.isArray(p.publish) && p.publish.length === 0);
    const order = [], seen = new Set();
    const visit = name => {
      if (seen.has(name)) return; seen.add(name);
      const p = byName[name]; if (!p) return;
      for (const d of p.dependencies)
        if (members.has(d.name)) visit(d.name);   // deps first
      order.push(p);
    };
    m.packages.map(p => p.name).sort().forEach(visit);
    for (const p of order) if (publishable(p)) console.log(p.name);
  ')"

  if [[ -z "$crates" ]]; then
    warn "no publishable crates (all marked publish = false) — skipping crates.io"
    return
  fi

  info "publishable, in order: $(echo "$crates" | tr '\n' ' ')"
  local dirty_flag=(); (( ALLOW_DIRTY )) && dirty_flag=(--allow-dirty)

  while IFS= read -r crate; do
    [[ -n "$crate" ]] || continue
    step "cargo publish -p $crate"
    run cargo publish -p "$crate" ${dirty_flag[@]+"${dirty_flag[@]}"}
    # crates.io needs a moment to index a new version before a dependent can build.
    if (( ! DRY_RUN )); then info "waiting for the index to settle…"; sleep 15; fi
  done <<< "$crates"

  ok "Rust crates published"
}

# ════════════════════════════════════════════════════════════════════════════════
# 2. NPM — build both packages (incl. the wasm they bundle) and npm publish them
# ════════════════════════════════════════════════════════════════════════════════
publish_npm() {
  step "npm — build + publish @visioncortex packages"

  if ! (( DRY_RUN )) && ! npm whoami >/dev/null 2>&1; then
    die "not logged in to npm (run: npm login)"
  fi

  # build:packages = sync README → build release wasm (web + node) → build both dists.
  run npm run build:packages

  # Scoped packages default to private on the registry; --access public makes them open.
  for pkg in @visioncortex/xsvg-viewer @visioncortex/xsvg-compile; do
    step "npm publish $pkg"
    run npm publish -w "$pkg" --access public
  done

  ok "npm packages published"
}

# ════════════════════════════════════════════════════════════════════════════════
# 3. RELEASE — attach the raw wasm compiler binary to a GitHub release
# ════════════════════════════════════════════════════════════════════════════════
publish_release() {
  step "GitHub release — attach wasm binary ($TAG)"

  local node_pkg="packages/xsvg-compile/pkg"

  # Build the wasm if a previous phase didn't already (npm's build:packages does).
  if [[ ! -d "$node_pkg" ]]; then
    info "wasm not built yet — building release wasm"
    run npm run wasm:node
  else
    info "reusing wasm already in $node_pkg"
  fi

  # Stage the assets under target/ (git-ignored). The npm packages already bundle
  # the wasm (xsvg-compile ships pkg/, xsvg-viewer inlines it into dist), so this is
  # only for non-npm consumers. The web and node targets emit the *same* .wasm (they
  # differ only in JS glue), so we attach a single binary + its checksum.
  local stage="target/release-assets"
  run rm -rf "$stage"
  run mkdir -p "$stage"

  if (( ! DRY_RUN )); then
    cp "$node_pkg"/*_bg.wasm "$stage/xsvg_wasm_bg.wasm"
    ( cd "$stage" && shasum -a 256 xsvg_wasm_bg.wasm > SHA256SUMS.txt )
  fi
  info "staged assets:"
  (( DRY_RUN )) && info "  (dry-run: would stage xsvg_wasm_bg.wasm + SHA256SUMS.txt)" \
                || ls -1 "$stage" | sed 's/^/    /'

  local assets=(
    "$stage/xsvg_wasm_bg.wasm"
    "$stage/SHA256SUMS.txt"
  )

  # Create the release on first run, otherwise upload/replace the assets on it.
  if (( DRY_RUN )); then
    info "would ensure release $TAG exists, then upload:"
    printf '    %s\n' "${assets[@]}"
  elif gh release view "$TAG" >/dev/null 2>&1; then
    info "release $TAG exists — uploading assets (--clobber)"
    run gh release upload "$TAG" "${assets[@]}" --clobber
  else
    info "creating release $TAG"
    run gh release create "$TAG" "${assets[@]}" \
      --title "xsvg $TAG" \
      --notes "xsvg $TAG — the raw WebAssembly compiler binary, for using xsvg outside npm (direct <script>/CDN, another language's wasm runtime, archival). Most users don't need this: the npm packages already bundle the same wasm — \`@visioncortex/xsvg-viewer\` (browser) inlines it, \`@visioncortex/xsvg-compile\` (Node) ships it."
  fi

  ok "GitHub release $TAG updated"
}

# ── run the selected phases ─────────────────────────────────────────────────────
(( DO_RUST ))    && publish_rust
(( DO_NPM ))     && publish_npm
(( DO_RELEASE )) && publish_release

step "done"
ok "publish complete${DRY_RUN:+ (dry-run)}"
