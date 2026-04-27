#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist/$(date +%Y%m%d%H%M%S)"

cd "$ROOT_DIR"

cargo fmt --all -- --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release

mkdir -p "$DIST_DIR"
cp target/release/singulari-world "$DIST_DIR/"
cp target/release/singulari-world-mcp "$DIST_DIR/"
cp README.md AGENTS.md LICENSE "$DIST_DIR/"
cp -R docs examples scripts "$DIST_DIR/"

echo "release artifacts: $DIST_DIR"
