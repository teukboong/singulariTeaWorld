#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVER_NAME="${SINGULARI_WORLD_MCP_NAME:-singulari-world}"
STORE_ROOT="${SINGULARI_WORLD_HOME:-}"
BIN="$ROOT_DIR/target/release/singulari-world-mcp"

cd "$ROOT_DIR"

cargo build --locked --release --bin singulari-world-mcp

if ! command -v codex >/dev/null 2>&1; then
  echo "codex CLI not found on PATH" >&2
  exit 1
fi

if [[ -n "$STORE_ROOT" ]]; then
  codex mcp add "$SERVER_NAME" --env "SINGULARI_WORLD_HOME=$STORE_ROOT" -- "$BIN"
else
  codex mcp add "$SERVER_NAME" -- "$BIN"
fi

codex mcp get "$SERVER_NAME"
