#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STORE_ROOT="${SINGULARI_WORLD_SMOKE_STORE:-$(mktemp -d "${TMPDIR:-/tmp}/singulari-world-smoke.XXXXXX")}"
WORLD_ID="smoke_$(date +%Y%m%d%H%M%S)_$$"
BIN="$ROOT_DIR/target/debug/singulari-world"

cd "$ROOT_DIR"

cargo build --locked

"$BIN" --store-root "$STORE_ROOT" start \
  --world-id "$WORLD_ID" \
  --seed-text "public alpha smoke fantasy world" \
  --json >/dev/null

"$BIN" --store-root "$STORE_ROOT" vn-packet \
  --world-id "$WORLD_ID" \
  --json >/dev/null

"$BIN" --store-root "$STORE_ROOT" agent-submit \
  --world-id "$WORLD_ID" \
  --input 1 \
  --json >/dev/null

"$BIN" --store-root "$STORE_ROOT" agent-next \
  --world-id "$WORLD_ID" \
  --json | grep -q '"player_input": "1"'

"$BIN" --store-root "$STORE_ROOT" visual-assets \
  --world-id "$WORLD_ID" \
  --json | grep -q '"codex_app_call"'

"$BIN" --store-root "$STORE_ROOT" validate \
  --world-id "$WORLD_ID" \
  --json >/dev/null

echo "smoke ok: world_id=$WORLD_ID store_root=$STORE_ROOT"
