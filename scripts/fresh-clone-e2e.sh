#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_PARENT="$(mktemp -d "${TMPDIR:-/tmp}/singulari-world-fresh-clone.XXXXXX")"
CLONE_DIR="$TMP_PARENT/repo"
STORE_ROOT="$TMP_PARENT/store"
WORLD_ID="fresh_clone_$(date +%Y%m%d%H%M%S)_$$"

cleanup() {
  if [[ "${SINGULARI_WORLD_KEEP_FRESH_CLONE:-0}" != "1" ]]; then
    rm -rf "$TMP_PARENT"
  else
    printf 'fresh clone kept: %s\n' "$TMP_PARENT" >&2
  fi
}
trap cleanup EXIT

cd "$ROOT_DIR"

if ! git diff --quiet || ! git diff --cached --quiet; then
  printf 'warning: fresh clone uses committed git state; local uncommitted edits are not included\n' >&2
fi

git clone --quiet -- "$ROOT_DIR" "$CLONE_DIR"
cd "$CLONE_DIR"

scripts/privacy-audit.sh
cargo build --locked
cargo check --locked --bin singulari-world-mcp
cargo check --locked --bin singulari-world-mcp-web

target/debug/singulari-world --store-root "$STORE_ROOT" start \
  --world-id "$WORLD_ID" \
  --seed-text "fresh clone public alpha smoke fantasy world" \
  --json >/dev/null

target/debug/singulari-world --store-root "$STORE_ROOT" vn-packet \
  --world-id "$WORLD_ID" \
  --json >/dev/null

target/debug/singulari-world --store-root "$STORE_ROOT" agent-submit \
  --world-id "$WORLD_ID" \
  --input 1 \
  --json >/dev/null

target/debug/singulari-world --store-root "$STORE_ROOT" agent-next \
  --world-id "$WORLD_ID" \
  --json | grep -q '"player_input": "1"'

target/debug/singulari-world --store-root "$STORE_ROOT" visual-assets \
  --world-id "$WORLD_ID" \
  --json | grep -q '"image_generation_call"'

target/debug/singulari-world --store-root "$STORE_ROOT" host-supervisor \
  --world-id "$WORLD_ID" \
  --json | grep -q '"status": "ready"'

target/debug/singulari-world --store-root "$STORE_ROOT" validate \
  --world-id "$WORLD_ID" \
  --json >/dev/null

printf 'fresh clone e2e ok: world_id=%s clone=%s store_root=%s\n' \
  "$WORLD_ID" "$CLONE_DIR" "$STORE_ROOT"
