#!/usr/bin/env bash
# Trunk post_build hook: shrink the release wasm with wasm-opt -Oz.
#
# Trunk's built-in wasm-opt doesn't pass the feature flags this binary needs
# (rustc emits bulk-memory / nontrapping-float-to-int / sign-ext), so we run
# wasm-opt ourselves with them enabled. Release builds only — dev `trunk serve`
# skips this and stays fast.
set -euo pipefail

[ "${TRUNK_PROFILE:-}" = "release" ] || exit 0

for dir in "${TRUNK_STAGING_DIR:-}" "${TRUNK_DIST_DIR:-}"; do
  [ -n "$dir" ] || continue
  wasm="$dir/usdt-wallet-web_bg.wasm"
  [ -f "$wasm" ] || continue

  before=$(wc -c <"$wasm")
  wasm-opt \
    --enable-bulk-memory \
    --enable-nontrapping-float-to-int \
    --enable-sign-ext \
    --enable-mutable-globals \
    --enable-reference-types \
    --enable-multivalue \
    -Oz "$wasm" -o "$wasm.tmp"
  mv "$wasm.tmp" "$wasm"
  after=$(wc -c <"$wasm")
  echo "wasm-opt -Oz: $((before / 1024)) KiB -> $((after / 1024)) KiB"
  exit 0
done
