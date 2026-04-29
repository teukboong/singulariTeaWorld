#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist/$(date +%Y%m%d%H%M%S)"

cd "$ROOT_DIR"

scripts/privacy-audit.sh
cargo fmt --all -- --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
(cd webgpt-mcp-checkout && cargo check --locked)

mkdir -p "$DIST_DIR"
cp target/release/singulari-world "$DIST_DIR/"
cp target/release/singulari-world-mcp "$DIST_DIR/"
cp README.md AGENTS.md LICENSE "$DIST_DIR/"
cp -R docs examples scripts webgpt-mcp-checkout "$DIST_DIR/"

echo "release artifacts: $DIST_DIR"
