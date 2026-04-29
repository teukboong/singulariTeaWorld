#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WEBGPT_DIR="$ROOT_DIR/webgpt-mcp-checkout"
WORKER_DIR="$WEBGPT_DIR/chatgpt-worker"

cd "$WORKER_DIR"
npm ci
npm run build

cd "$WEBGPT_DIR"
cargo build --locked --release

printf 'webgpt runtime ready: %s\n' "$WEBGPT_DIR/scripts/webgpt-mcp.sh"
