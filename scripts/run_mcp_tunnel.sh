#!/usr/bin/env bash
set -euo pipefail
umask 077

cd "$(dirname "$0")/.."

export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"

MODE_OVERRIDE="${SINGULARI_WORLD_TUNNEL:-}"

set -a
if [[ -f ".env" ]]; then
  # shellcheck disable=SC1091
  source ".env"
fi
set +a

if [[ -n "${MODE_OVERRIDE}" ]]; then
  export SINGULARI_WORLD_TUNNEL="${MODE_OVERRIDE}"
fi

MODE="${SINGULARI_WORLD_TUNNEL:-cloudflared}"
TARGET_URL="${SINGULARI_WORLD_TUNNEL_TARGET_URL:-http://127.0.0.1:4187}"
STATE_FILE="${SINGULARI_WORLD_TUNNEL_STATE_FILE:-.runtime/mcp_tunnel_base_url.txt}"
PENDING_FILE="${SINGULARI_WORLD_TUNNEL_PENDING_FILE:-.runtime/mcp_tunnel_origin_pending.txt}"
RETRY_BACKOFF_SEC="${SINGULARI_WORLD_TUNNEL_RETRY_BACKOFF_SEC:-5}"
RETRY_BACKOFF_MAX_SEC="${SINGULARI_WORLD_TUNNEL_RETRY_BACKOFF_MAX_SEC:-120}"
ORIGIN_RETRY_INTERVAL_SEC="${SINGULARI_WORLD_TUNNEL_ORIGIN_RETRY_INTERVAL_SEC:-30}"
STABLE_RUN_RESET_SEC="${SINGULARI_WORLD_TUNNEL_STABLE_RUN_RESET_SEC:-180}"
BACKOFF_SEC="${RETRY_BACKOFF_SEC}"

mkdir -p "$(dirname "${STATE_FILE}")" "$(dirname "${PENDING_FILE}")"
touch "${STATE_FILE}" "${PENDING_FILE}"

_now_epoch() {
  date +%s
}

_trimmed_file_value() {
  local path="$1"
  if [[ ! -f "${path}" ]]; then
    return 0
  fi
  tr -d '\r\n' < "${path}" 2>/dev/null || true
}

_state_url() {
  _trimmed_file_value "${STATE_FILE}"
}

_pending_url() {
  _trimmed_file_value "${PENDING_FILE}"
}

_set_pending_url() {
  printf '%s' "$1" > "${PENDING_FILE}"
}

_clear_pending_url() {
  : > "${PENDING_FILE}"
}

_apply_public_url() {
  local url="$1"
  local frontdoor="${SINGULARI_WORLD_FRONTDOOR_URL:-}"
  frontdoor="${frontdoor%/}"

  if [[ -z "${frontdoor}" || -z "${SINGULARI_WORLD_FRONTDOOR_UPDATE_SECRET:-}" ]]; then
    echo "WARN: Missing SINGULARI_WORLD_FRONTDOOR_URL or SINGULARI_WORLD_FRONTDOOR_UPDATE_SECRET."
    echo "      Public tunnel URL is ${url}"
    return 1
  fi

  echo "Updating Singulari front door origin via Worker: ${frontdoor}"
  if curl -fsS -X POST "${frontdoor}/_singulari/origin" \
    -H "Content-Type: application/json" \
    -H "X-Singulari-Origin-Update-Secret: ${SINGULARI_WORLD_FRONTDOOR_UPDATE_SECRET}" \
    --data "{\"origin\":\"${url}\"}" >/dev/null; then
    echo "OK: Worker KV updated"
    return 0
  fi

  echo "WARN: Failed to update Worker KV. Check front door URL, secret, and Worker deployment."
  return 1
}

_commit_synced_url() {
  local url="$1"
  printf '%s' "${url}" > "${STATE_FILE}"
  _clear_pending_url
  echo "Origin sync committed: ${url}"
  if [[ -n "${SINGULARI_WORLD_FRONTDOOR_URL:-}" ]]; then
    echo "ChatGPT MCP URL: ${SINGULARI_WORLD_FRONTDOOR_URL%/}/mcp"
  fi
}

_sync_url_if_needed() {
  local url="$1"
  local last pending
  last="$(_state_url)"
  pending="$(_pending_url)"

  if [[ "${url}" == "${last}" && "${pending}" != "${url}" ]]; then
    return 0
  fi

  if [[ "${url}" != "${last}" ]]; then
    echo "Detected new public URL: ${url}"
  else
    echo "Origin sync still pending for current URL: ${url} (retry)"
  fi

  if _apply_public_url "${url}"; then
    _commit_synced_url "${url}"
    return 0
  fi

  _set_pending_url "${url}"
  return 1
}

_retry_pending_origin_if_any() {
  local pending now last_retry
  pending="$(_pending_url)"
  if [[ -z "${pending}" ]]; then
    return 0
  fi

  now="$(_now_epoch)"
  last_retry="${ORIGIN_LAST_RETRY_AT_EPOCH:-0}"
  if [[ "${last_retry}" -gt 0 && $((now - last_retry)) -lt "${ORIGIN_RETRY_INTERVAL_SEC}" ]]; then
    return 0
  fi

  export ORIGIN_LAST_RETRY_AT_EPOCH="${now}"
  echo "[origin-sync] retry pending URL: ${pending}"
  if _apply_public_url "${pending}"; then
    _commit_synced_url "${pending}"
  else
    echo "[origin-sync] retry failed; will retry in ${ORIGIN_RETRY_INTERVAL_SEC}s"
  fi
}

_start_pending_sync_loop() {
  while true; do
    _retry_pending_origin_if_any || true
    sleep "${ORIGIN_RETRY_INTERVAL_SEC}"
  done
}

_cleanup_background() {
  if [[ -n "${ORIGIN_SYNC_PID:-}" ]]; then
    kill "${ORIGIN_SYNC_PID}" >/dev/null 2>&1 || true
  fi
}

_reset_backoff() {
  BACKOFF_SEC="${RETRY_BACKOFF_SEC}"
}

_increase_backoff() {
  if [[ "${BACKOFF_SEC}" -lt "${RETRY_BACKOFF_MAX_SEC}" ]]; then
    BACKOFF_SEC=$((BACKOFF_SEC * 2))
    if [[ "${BACKOFF_SEC}" -gt "${RETRY_BACKOFF_MAX_SEC}" ]]; then
      BACKOFF_SEC="${RETRY_BACKOFF_MAX_SEC}"
    fi
  fi
}

_handle_tunnel_exit() {
  local mode="$1"
  local exit_code="$2"
  local run_elapsed="$3"

  if [[ "${run_elapsed}" -ge "${STABLE_RUN_RESET_SEC}" ]]; then
    _reset_backoff
  fi

  if [[ "${exit_code}" -eq 0 ]]; then
    echo "[${mode}] tunnel exited cleanly; restarting in 1s"
    _reset_backoff
    sleep 1
    return
  fi

  echo "[${mode}] tunnel failed (exit=${exit_code}); retrying in ${BACKOFF_SEC}s"
  sleep "${BACKOFF_SEC}"
  _increase_backoff
}

_start_pending_sync_loop &
ORIGIN_SYNC_PID=$!
trap _cleanup_background EXIT INT TERM

if [[ "${MODE}" == "cloudflared" ]]; then
  BIN="${SINGULARI_WORLD_CLOUDFLARED_BIN:-cloudflared}"
  if ! command -v "${BIN}" >/dev/null 2>&1; then
    echo "cloudflared not found. Install it first: brew install cloudflared"
    exit 1
  fi

  echo "Starting cloudflared quick tunnel -> ${TARGET_URL}"
  echo "State file: ${STATE_FILE}"
  while true; do
    _retry_pending_origin_if_any || true
    run_started_at="$(_now_epoch)"
    set +e
    "${BIN}" tunnel --no-autoupdate --url "${TARGET_URL}" 2>&1 | while IFS= read -r line; do
      printf '%s\n' "${line}"
      url="$(printf '%s' "${line}" | grep -oE 'https://[A-Za-z0-9.-]+\.trycloudflare\.com' | head -n 1 || true)"
      if [[ -n "${url}" ]]; then
        _sync_url_if_needed "${url}" || true
      fi
    done
    exit_code="${PIPESTATUS[0]}"
    run_elapsed="$(( $(_now_epoch) - run_started_at ))"
    set -e
    _handle_tunnel_exit "cloudflared" "${exit_code}" "${run_elapsed}"
  done
fi

echo "Unsupported SINGULARI_WORLD_TUNNEL=${MODE}"
echo "Supported: cloudflared"
exit 1
