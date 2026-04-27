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

"$BIN" --store-root "$STORE_ROOT" host-worker \
  --world-id "$WORLD_ID" \
  --text-backend manual \
  --no-visual-jobs \
  --once | grep -q '"event":"manual_agent_turn_required"'

CLAIM_FILE="$STORE_ROOT/claim.json"
"$BIN" --store-root "$STORE_ROOT" visual-job-claim \
  --world-id "$WORLD_ID" \
  --slot menu_background \
  --claimed-by smoke-local \
  --json >"$CLAIM_FILE"

CLAIM_ID="$(sed -n 's/.*"claim_id": "\(.*\)".*/\1/p' "$CLAIM_FILE" | head -n 1)"
DESTINATION_PATH="$(sed -n 's/.*"destination_path": "\(.*\)".*/\1/p' "$CLAIM_FILE" | head -n 1)"

if [[ -z "$CLAIM_ID" || -z "$DESTINATION_PATH" ]]; then
  echo "failed to parse visual job claim" >&2
  exit 1
fi

GENERATED_PATH="$STORE_ROOT/generated.png"
printf '\211PNG\r\n\032\nsmoke-png' >"$GENERATED_PATH"

"$BIN" --store-root "$STORE_ROOT" visual-job-complete \
  --world-id "$WORLD_ID" \
  --slot menu_background \
  --claim-id "$CLAIM_ID" \
  --generated-path "$GENERATED_PATH" \
  --json >/dev/null

test -f "$DESTINATION_PATH"

"$BIN" --store-root "$STORE_ROOT" visual-assets \
  --world-id "$WORLD_ID" \
  --json | grep -q '"exists": true'

"$BIN" --store-root "$STORE_ROOT" validate \
  --world-id "$WORLD_ID" \
  --json >/dev/null

echo "smoke ok: world_id=$WORLD_ID store_root=$STORE_ROOT"
