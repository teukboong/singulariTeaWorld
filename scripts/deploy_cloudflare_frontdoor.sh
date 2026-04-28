#!/usr/bin/env bash
set -euo pipefail
umask 077

cd "$(dirname "$0")/.."

export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"

set -a
if [[ -f ".env" ]]; then
  # shellcheck disable=SC1091
  source ".env"
fi
set +a

WORKER_DIR="cloudflare/worker"
BASE_CONFIG="${WORKER_DIR}/wrangler.toml"
GENERATED_CONFIG="${WORKER_DIR}/.wrangler.generated.toml"
PLACEHOLDER="REPLACE_WITH_KV_NAMESPACE_ID"

if ! command -v npx >/dev/null 2>&1; then
  echo "npx not found. Install Node.js first."
  exit 1
fi

CONFIG="${BASE_CONFIG}"
if grep -q "${PLACEHOLDER}" "${BASE_CONFIG}"; then
  KV_ID="${SINGULARI_WORLD_CF_KV_NAMESPACE_ID:-}"
  if [[ -z "${KV_ID}" || "${KV_ID}" == "replace-with-workers-kv-namespace-id" ]]; then
    cat >&2 <<EOF
Missing SINGULARI_WORLD_CF_KV_NAMESPACE_ID.

Create a Workers KV namespace, then put its id in .env:

  cd ${WORKER_DIR}
  npx wrangler kv namespace create SINGULARI_WORLD_KV

  SINGULARI_WORLD_CF_KV_NAMESPACE_ID=<id>

EOF
    exit 2
  fi
  sed "s/${PLACEHOLDER}/${KV_ID}/g" "${BASE_CONFIG}" > "${GENERATED_CONFIG}"
  CONFIG="${GENERATED_CONFIG}"
fi

(
  cd "${WORKER_DIR}"
  npx wrangler deploy --config "$(basename "${CONFIG}")"
)

if [[ -n "${SINGULARI_WORLD_FRONTDOOR_UPDATE_SECRET:-}" && "${SINGULARI_WORLD_FRONTDOOR_UPDATE_SECRET}" != "replace-with-random-secret" ]]; then
  (
    cd "${WORKER_DIR}"
    printf '%s' "${SINGULARI_WORLD_FRONTDOOR_UPDATE_SECRET}" \
      | npx wrangler secret put ORIGIN_UPDATE_SECRET --config "$(basename "${CONFIG}")"
  )
else
  cat >&2 <<'EOF'
WARN: SINGULARI_WORLD_FRONTDOOR_UPDATE_SECRET is not set.
Set Worker secret manually before running scripts/run_mcp_tunnel.sh:

  cd cloudflare/worker
  npx wrangler secret put ORIGIN_UPDATE_SECRET

EOF
fi

if [[ -n "${SINGULARI_WORLD_FRONTDOOR_URL:-}" ]]; then
  echo "ChatGPT MCP URL: ${SINGULARI_WORLD_FRONTDOOR_URL%/}/mcp"
else
  echo "Set SINGULARI_WORLD_FRONTDOOR_URL in .env to the deployed Worker URL."
fi
