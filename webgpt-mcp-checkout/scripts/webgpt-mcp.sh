#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${HESPERIDES_REPO_ROOT:-$(cd "${SCRIPT_DIR}/.." && pwd)}"
RELEASE_BIN="${WEBGPT_MCP_BIN:-${REPO_ROOT}/target/release/webgpt-mcp}"
DEBUG_BIN="${WEBGPT_MCP_DEBUG_BIN:-${REPO_ROOT}/target/debug/webgpt-mcp}"
DEFAULT_MCP_PROFILE_DIR="${HOME}/.hesperides/chatgpt-chrome-profile-mcp"
INTERACTIVE_PROFILE_DIR="${WEBGPT_INTERACTIVE_PROFILE_DIR:-${HOME}/.hesperides/chatgpt-chrome-profile}"
MANUAL_PROFILE_DIR="${WEBGPT_MCP_MANUAL_PROFILE_DIR:-${HOME}/.hesperides/chatgpt-chrome-profile-manual}"
INTERACTIVE_SNAPSHOT_DIR="${WEBGPT_MCP_BOOTSTRAP_SNAPSHOT_DIR:-${INTERACTIVE_PROFILE_DIR}-snapshot}"
DEFAULT_CHATGPT_URL="${WEBGPT_MCP_CHATGPT_URL:-https://chatgpt.com/}"
DEFAULT_CHROME_USER_DATA_DIR="${HOME}/Library/Application Support/Google/Chrome"
CHROME_USER_DATA_DIR="${WEBGPT_MCP_BOOTSTRAP_CHROME_USER_DATA_DIR:-${DEFAULT_CHROME_USER_DATA_DIR}}"
DEFAULT_CDP_PORT="${WEBGPT_MCP_CDP_PORT:-9228}"
DEFAULT_CDP_URL="http://127.0.0.1:${DEFAULT_CDP_PORT}"
CDP_SESSION_HELPER="${REPO_ROOT}/scripts/webgpt-cdp-session.sh"
AUTO_CDP_DISABLED="${WEBGPT_MCP_DISABLE_AUTO_CDP:-0}"
BOOTSTRAP_MARKER_FILENAME=".webgpt-mcp-bootstrap.json"

uses_cdp_attach() {
    [[ -n "${WEBGPT_MCP_CDP_URL:-}" ]] && return 0
    return 1
}

cdp_endpoint_ready() {
    command -v curl >/dev/null 2>&1 || return 1
    curl -fsS "${DEFAULT_CDP_URL}/json/version" >/dev/null 2>&1
}

cdp_chatgpt_surface_ready() {
    command -v curl >/dev/null 2>&1 || return 1
    command -v jq >/dev/null 2>&1 || return 1
    curl -fsS "${DEFAULT_CDP_URL}/json/list" 2>/dev/null |
        jq -e --arg target_url "${DEFAULT_CHATGPT_URL}" '
            any(
                .[];
                ((.url // "") | startswith($target_url))
                and ((.title // "") != "")
                and ((.title // "") != "Just a moment...")
                and ((.title // "") != "잠시만 기다리십시오…")
            )
        ' >/dev/null
}

ensure_default_cdp_attach() {
    [[ "${AUTO_CDP_DISABLED}" == "1" ]] && return 1
    uses_cdp_attach && return 0

    if ! cdp_endpoint_ready; then
        [[ -x "${CDP_SESSION_HELPER}" ]] || return 1
        "${CDP_SESSION_HELPER}" start >/dev/null
    fi

    cdp_endpoint_ready || return 1
    local attempt
    for attempt in $(seq 1 30); do
        if cdp_chatgpt_surface_ready; then
            break
        fi
        sleep 1
    done
    export WEBGPT_MCP_CDP_URL="${DEFAULT_CDP_URL}"
    echo "webgpt-mcp: using auto-managed CDP session at ${DEFAULT_CDP_URL}" >&2
    return 0
}

profile_marker_mtime() {
    local marker="$1/Local State"
    [[ -f "$marker" ]] || {
        printf '0\n'
        return 0
    }
    stat -f '%m' "$marker" 2>/dev/null || stat -c '%Y' "$marker" 2>/dev/null || printf '0\n'
}

has_reusable_profile() {
    local dir="$1"
    [[ -f "${dir}/Local State" ]] || return 1
    [[ -d "${dir}/Default" ]] && return 0
    find "${dir}" -maxdepth 1 -type d -name 'Profile *' -print -quit | grep -q .
}

has_chatgpt_session_token() {
    local profile_dir="$1"
    local cookies_path="${profile_dir}/Default/Cookies"
    local tmp_db
    [[ -f "${cookies_path}" ]] || {
        printf '0\n'
        return 0
    }
    command -v sqlite3 >/dev/null 2>&1 || {
        printf '0\n'
        return 0
    }

    tmp_db="$(mktemp "${TMPDIR:-/tmp}/webgpt-cookie-check.XXXXXX")" || {
        printf '0\n'
        return 0
    }
    if ! cp -p "${cookies_path}" "${tmp_db}" 2>/dev/null; then
        rm -f "${tmp_db}"
        printf '0\n'
        return 0
    fi

    if sqlite3 -readonly "${tmp_db}" \
        "SELECT 1 FROM cookies WHERE host_key LIKE '%chatgpt.com' AND name = '__Secure-next-auth.session-token' LIMIT 1;" \
        2>/dev/null | grep -qx '1'; then
        printf '1\n'
    else
        printf '0\n'
    fi
    rm -f "${tmp_db}"
}

sync_profile_tree() {
    local source_dir="$1"
    mkdir -p "${WEBGPT_MCP_PROFILE_DIR}"
    rsync -a \
        --exclude 'DevToolsActivePort' \
        --exclude 'SingletonCookie' \
        --exclude 'SingletonLock' \
        --exclude 'SingletonSocket' \
        --exclude 'BrowserMetrics*' \
        --exclude 'Crashpad*' \
        --exclude 'ShaderCache*' \
        --exclude 'GrShaderCache*' \
        "${source_dir}/" "${WEBGPT_MCP_PROFILE_DIR}/"
}

record_bootstrap_metadata() {
    local source_label="$1"
    local source_dir="$2"
    command -v jq >/dev/null 2>&1 || {
        echo "webgpt-mcp: bootstrap metadata skipped because jq is unavailable" >&2
        return 1
    }
    jq -n \
        --arg source "${source_label}" \
        --arg source_profile_dir "${source_dir}" \
        --arg recorded_at "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" \
        '{source: $source, source_profile_dir: $source_profile_dir, recorded_at: $recorded_at}' \
        >"${WEBGPT_MCP_PROFILE_DIR}/${BOOTSTRAP_MARKER_FILENAME}"
}

bootstrap_mcp_profile_from() {
    local source_dir="$1"
    local source_label="$2"
    local source_mtime
    local target_mtime
    local source_has_session
    local target_has_session

    [[ "${source_dir}" == "${WEBGPT_MCP_PROFILE_DIR}" ]] && return 1
    has_reusable_profile "${source_dir}" || return 1

    source_mtime="$(profile_marker_mtime "${source_dir}")"
    target_mtime="$(profile_marker_mtime "${WEBGPT_MCP_PROFILE_DIR}")"
    source_has_session="$(has_chatgpt_session_token "${source_dir}")"
    target_has_session="$(has_chatgpt_session_token "${WEBGPT_MCP_PROFILE_DIR}")"
    if [[ "${source_has_session}" != "1" ]]; then
        return 1
    fi
    if [[ "${target_has_session}" == "1" && "${target_mtime}" -gt 0 && "${source_mtime}" -le "${target_mtime}" ]]; then
        return 1
    fi

    if ! command -v rsync >/dev/null 2>&1; then
        echo "webgpt-mcp: bootstrap skipped because rsync is unavailable" >&2
        return 1
    fi

    sync_profile_tree "${source_dir}"
    record_bootstrap_metadata "${source_label}" "${source_dir}"
    echo "webgpt-mcp: bootstrapped MCP profile from ${source_label}" >&2
    return 0
}

bootstrap_mcp_profile() {
    bootstrap_mcp_profile_from "${CHROME_USER_DATA_DIR}" "chrome user data" && return 0
    bootstrap_mcp_profile_from "${MANUAL_PROFILE_DIR}" "manual profile" && return 0
    bootstrap_mcp_profile_from "${INTERACTIVE_SNAPSHOT_DIR}" "interactive snapshot" && return 0
    return 1
}

is_protected_bootstrap_target() {
    [[ "${WEBGPT_MCP_PROFILE_DIR}" == "${MANUAL_PROFILE_DIR}" ]] && return 0
    [[ "${WEBGPT_MCP_PROFILE_DIR}" == "${INTERACTIVE_SNAPSHOT_DIR}" ]] && return 0
    return 1
}

resolve_bin() {
    if [[ -x "${RELEASE_BIN}" && -x "${DEBUG_BIN}" ]]; then
        local release_mtime
        local debug_mtime
        release_mtime="$(stat -f '%m' "${RELEASE_BIN}")"
        debug_mtime="$(stat -f '%m' "${DEBUG_BIN}")"
        if [[ "${debug_mtime}" -gt "${release_mtime}" ]]; then
            printf '%s\n' "${DEBUG_BIN}"
        else
            printf '%s\n' "${RELEASE_BIN}"
        fi
        return 0
    fi
    if [[ -x "${RELEASE_BIN}" ]]; then
        printf '%s\n' "${RELEASE_BIN}"
        return 0
    fi
    if [[ -x "${DEBUG_BIN}" ]]; then
        printf '%s\n' "${DEBUG_BIN}"
        return 0
    fi
    return 1
}

if [[ -z "${WEBGPT_MCP_PROFILE_DIR:-}" ]]; then
    export WEBGPT_MCP_PROFILE_DIR="${DEFAULT_MCP_PROFILE_DIR}"
fi

if [[ "${WEBGPT_MCP_PROFILE_DIR}" == "${INTERACTIVE_PROFILE_DIR}" ]]; then
    echo "webgpt-mcp: fail-closed because WEBGPT_MCP_PROFILE_DIR matches interactive profile dir" >&2
    echo "  WEBGPT_MCP_PROFILE_DIR=${WEBGPT_MCP_PROFILE_DIR}" >&2
    echo "  WEBGPT_INTERACTIVE_PROFILE_DIR=${INTERACTIVE_PROFILE_DIR}" >&2
    exit 1
fi

ensure_default_cdp_attach || true

if uses_cdp_attach; then
    echo "webgpt-mcp: bootstrap skipped because CDP attach mode is active" >&2
elif is_protected_bootstrap_target; then
    echo "webgpt-mcp: bootstrap skipped because target profile is a protected source profile" >&2
else
    bootstrap_mcp_profile || true
fi

if MCP_BIN="$(resolve_bin)"; then
    exec "${MCP_BIN}" "$@"
fi

cd "${REPO_ROOT}"
exec cargo run -q -p webgpt-mcp -- "$@"
