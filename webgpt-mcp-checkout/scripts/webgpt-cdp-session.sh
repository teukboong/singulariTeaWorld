#!/bin/bash
set -euo pipefail

DEFAULT_PORT="${WEBGPT_MCP_CDP_PORT:-9228}"
STATE_DIR="${WEBGPT_MCP_CDP_STATE_DIR:-${HOME}/.hesperides/webgpt-cdp-session/${DEFAULT_PORT}}"
PID_FILE="${STATE_DIR}/chrome.pid"
LOG_FILE="${STATE_DIR}/chrome.log"
DEFAULT_URL="${WEBGPT_MCP_CHATGPT_URL:-https://chatgpt.com/}"
MANUAL_PROFILE_DIR="${WEBGPT_MCP_MANUAL_PROFILE_DIR:-${HOME}/.hesperides/chatgpt-chrome-profile-manual}"
DEFAULT_CHROME_USER_DATA_DIR="${HOME}/Library/Application Support/Google/Chrome"
CHROME_USER_DATA_DIR="${WEBGPT_MCP_BOOTSTRAP_CHROME_USER_DATA_DIR:-${DEFAULT_CHROME_USER_DATA_DIR}}"
DEFAULT_MANUAL_SOURCE_PROFILE_DIR="${HOME}/.hesperides/chatgpt-chrome-profile-manual"
BOOTSTRAP_SOURCE_PROFILE_DIRS="${WEBGPT_MCP_BOOTSTRAP_SOURCE_PROFILE_DIRS:-${DEFAULT_MANUAL_SOURCE_PROFILE_DIR}:${CHROME_USER_DATA_DIR}}"
CHROME_BIN="${WEBGPT_MCP_CHROME_BIN:-}"
COMMAND="${1:-status}"

resolve_chrome_bin() {
    if [[ -n "${CHROME_BIN}" && -x "${CHROME_BIN}" ]]; then
        printf '%s\n' "${CHROME_BIN}"
        return 0
    fi

    local candidate
    for candidate in \
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
        "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary" \
        "/Applications/Chromium.app/Contents/MacOS/Chromium"
    do
        if [[ -x "${candidate}" ]]; then
            printf '%s\n' "${candidate}"
            return 0
        fi
    done

    return 1
}

resolve_chrome_app_name() {
    local chrome_bin
    chrome_bin="$(resolve_chrome_bin)" || return 1
    case "${chrome_bin}" in
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
            printf '%s\n' "Google Chrome"
            ;;
        "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary")
            printf '%s\n' "Google Chrome Canary"
            ;;
        "/Applications/Chromium.app/Contents/MacOS/Chromium")
            printf '%s\n' "Chromium"
            ;;
        *)
            return 1
            ;;
    esac
}

listener_pids() {
    lsof -tiTCP:"${DEFAULT_PORT}" -sTCP:LISTEN 2>/dev/null || true
}

cdp_endpoint_ready() {
    command -v curl >/dev/null 2>&1 || return 1
    curl -fsS "http://127.0.0.1:${DEFAULT_PORT}/json/version" >/dev/null 2>&1
}

matching_pids() {
    WEBGPT_MATCH_PORT="${DEFAULT_PORT}" WEBGPT_MATCH_PROFILE="${MANUAL_PROFILE_DIR}" \
        awk '
            BEGIN {
                port = "--remote-debugging-port=" ENVIRON["WEBGPT_MATCH_PORT"]
                profile = "--user-data-dir=" ENVIRON["WEBGPT_MATCH_PROFILE"]
            }
            index($0, port) && index($0, profile) && $1 ~ /^[0-9]+$/ { print $1 }
        ' < <(ps -axo pid=,command= 2>/dev/null)
}

is_running() {
    [[ -n "$(matching_pids)" ]] && cdp_endpoint_ready && return 0
    return 1
}

fail_if_port_owned_by_other_process() {
    [[ -z "$(listener_pids)" ]] && return 0
    [[ -n "$(matching_pids)" ]] && return 0
    echo "webgpt-cdp-session: port ${DEFAULT_PORT} is already in use by a non-matching process" >&2
    echo "  expected_profile_dir=${MANUAL_PROFILE_DIR}" >&2
    echo "  listener_pids=$(listener_pids | paste -sd, -)" >&2
    exit 1
}

has_reusable_profile() {
    local dir="$1"
    [[ -f "${dir}/Local State" ]] || return 1
    [[ -d "${dir}/Default" ]] && return 0
    find "${dir}" -maxdepth 1 -type d -name 'Profile *' -print -quit | grep -q .
}

profile_marker_mtime() {
    local marker="$1/Local State"
    [[ -f "$marker" ]] || {
        printf '0\n'
        return 0
    }
    stat -f '%m' "$marker" 2>/dev/null || stat -c '%Y' "$marker" 2>/dev/null || printf '0\n'
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
    mkdir -p "${MANUAL_PROFILE_DIR}"
    rsync -a \
        --exclude 'DevToolsActivePort' \
        --exclude 'SingletonCookie' \
        --exclude 'SingletonLock' \
        --exclude 'SingletonSocket' \
        --exclude 'BrowserMetrics*' \
        --exclude 'Crashpad*' \
        --exclude 'ShaderCache*' \
        --exclude 'GrShaderCache*' \
        "${source_dir}/" "${MANUAL_PROFILE_DIR}/"
}

seed_manual_profile_from_source_if_needed() {
    local source_dir="$1"
    local source_mtime
    local manual_mtime
    local source_has_session
    local manual_has_session

    [[ "${source_dir}" != "${MANUAL_PROFILE_DIR}" ]] || return 1
    has_reusable_profile "${source_dir}" || return 1
    source_has_session="$(has_chatgpt_session_token "${source_dir}")"
    [[ "${source_has_session}" == "1" ]] || return 1

    manual_has_session="$(has_chatgpt_session_token "${MANUAL_PROFILE_DIR}")"
    source_mtime="$(profile_marker_mtime "${source_dir}")"
    manual_mtime="$(profile_marker_mtime "${MANUAL_PROFILE_DIR}")"
    if [[ "${manual_has_session}" == "1" && "${manual_mtime}" -gt 0 && "${source_mtime}" -le "${manual_mtime}" ]]; then
        return 0
    fi

    if ! command -v rsync >/dev/null 2>&1; then
        echo "webgpt-cdp-session: manual profile seed failed because rsync is unavailable" >&2
        return 2
    fi

    sync_profile_tree "${source_dir}"
    echo "webgpt-cdp-session: seeded manual profile from ${source_dir}" >&2
    return 0
}

seed_manual_profile_from_chrome_if_needed() {
    local source_dir
    local seeded=1

    IFS=':' read -r -a source_dirs <<< "${BOOTSTRAP_SOURCE_PROFILE_DIRS}"
    for source_dir in "${source_dirs[@]}"; do
        [[ -n "${source_dir}" ]] || continue
        if seed_manual_profile_from_source_if_needed "${source_dir}"; then
            seeded=0
            break
        fi
    done
    if [[ "${seeded}" == "0" ]]; then
        return 0
    fi

    has_reusable_profile "${CHROME_USER_DATA_DIR}" || return 0
    source_has_session="$(has_chatgpt_session_token "${CHROME_USER_DATA_DIR}")"
    [[ "${source_has_session}" == "1" ]] || return 0

    manual_has_session="$(has_chatgpt_session_token "${MANUAL_PROFILE_DIR}")"
    source_mtime="$(profile_marker_mtime "${CHROME_USER_DATA_DIR}")"
    manual_mtime="$(profile_marker_mtime "${MANUAL_PROFILE_DIR}")"
    if [[ "${manual_has_session}" == "1" && "${manual_mtime}" -gt 0 && "${source_mtime}" -le "${manual_mtime}" ]]; then
        return 0
    fi

    if ! command -v rsync >/dev/null 2>&1; then
        echo "webgpt-cdp-session: manual profile seed skipped because rsync is unavailable" >&2
        return 0
    fi

    sync_profile_tree "${CHROME_USER_DATA_DIR}"
    echo "webgpt-cdp-session: seeded manual profile from chrome user data" >&2
}

reap_stale_matching_session() {
    if [[ -z "$(matching_pids)" ]]; then
        return 0
    fi
    if cdp_endpoint_ready; then
        return 0
    fi

    echo "webgpt-cdp-session: reaping stale Chrome session for port ${DEFAULT_PORT}" >&2
    local pid
    while read -r pid; do
        [[ -n "${pid}" ]] || continue
        kill "${pid}" 2>/dev/null || true
    done < <(matching_pids | awk '!seen[$0]++')
    sleep 1
    while read -r pid; do
        [[ -n "${pid}" ]] || continue
        kill -9 "${pid}" 2>/dev/null || true
    done < <(matching_pids | awk '!seen[$0]++')
    rm -f "${PID_FILE}"
}

minimize_chrome_windows() {
    [[ "$(uname -s)" == "Darwin" ]] || return 0
    command -v osascript >/dev/null 2>&1 || return 0
    local chrome_app_name
    chrome_app_name="$(resolve_chrome_app_name)" || return 0
    osascript >/dev/null 2>&1 <<EOF || true
tell application "System Events"
  if exists process "${chrome_app_name}" then
    tell process "${chrome_app_name}"
      repeat with w in windows
        try
          set value of attribute "AXMinimized" of w to true
        end try
      end repeat
    end tell
  end if
end tell
EOF
}

current_pid_label() {
    local pid
    pid="$(listener_pids | head -n 1)"
    if [[ -n "${pid}" ]]; then
        printf '%s\n' "${pid}"
        return 0
    fi

    pid="$(matching_pids | head -n 1)"
    if [[ -n "${pid}" ]]; then
        printf '%s\n' "${pid}"
    fi
}

show_status() {
    if is_running; then
        local pid
        pid="$(current_pid_label)"
        cat <<EOF
status=running
pid=${pid}
cdp_url=http://127.0.0.1:${DEFAULT_PORT}
profile_dir=${MANUAL_PROFILE_DIR}
log_file=${LOG_FILE}
EOF
    else
        cat <<EOF
status=stopped
cdp_url=http://127.0.0.1:${DEFAULT_PORT}
profile_dir=${MANUAL_PROFILE_DIR}
log_file=${LOG_FILE}
EOF
    fi
}

start_session() {
    mkdir -p "${STATE_DIR}"

    if is_running; then
        show_status
        return 0
    fi
    reap_stale_matching_session
    fail_if_port_owned_by_other_process

    seed_manual_profile_from_chrome_if_needed

    local chrome_bin
    chrome_bin="$(resolve_chrome_bin)" || {
        echo "webgpt-cdp-session: Chrome executable not found" >&2
        exit 1
    }

    if [[ "$(uname -s)" == "Darwin" ]] && chrome_app_name="$(resolve_chrome_app_name)"; then
        nohup open -na "${chrome_app_name}" --args \
            --remote-debugging-port="${DEFAULT_PORT}" \
            --user-data-dir="${MANUAL_PROFILE_DIR}" \
            --no-first-run \
            --no-default-browser-check \
            --start-minimized \
            --new-window \
            "${DEFAULT_URL}" \
            >"${LOG_FILE}" 2>&1 &
    else
        nohup "${chrome_bin}" \
            --remote-debugging-port="${DEFAULT_PORT}" \
            --user-data-dir="${MANUAL_PROFILE_DIR}" \
            --no-first-run \
            --no-default-browser-check \
            --start-minimized \
            --new-window \
            "${DEFAULT_URL}" \
            >"${LOG_FILE}" 2>&1 &
    fi
    echo "$!" > "${PID_FILE}"

    local attempt
    for attempt in $(seq 1 10); do
        if is_running; then
            minimize_chrome_windows
            show_status
            return 0
        fi
        sleep 1
    done

    echo "webgpt-cdp-session: Chrome did not expose a CDP session on port ${DEFAULT_PORT}" >&2
    tail -n 40 "${LOG_FILE}" >&2 || true
    exit 1
}

stop_session() {
    if ! is_running; then
        rm -f "${PID_FILE}"
        show_status
        return 0
    fi

    local pid
    while read -r pid; do
        [[ -n "${pid}" ]] || continue
        kill "${pid}" 2>/dev/null || true
    done < <( { listener_pids; matching_pids; } | awk '!seen[$0]++' )
    sleep 2
    while read -r pid; do
        [[ -n "${pid}" ]] || continue
        kill -9 "${pid}" 2>/dev/null || true
    done < <( { listener_pids; matching_pids; } | awk '!seen[$0]++' )
    rm -f "${PID_FILE}"
    show_status
}

case "${COMMAND}" in
    start)
        start_session
        ;;
    stop)
        stop_session
        ;;
    status)
        show_status
        ;;
    *)
        echo "usage: $0 [start|stop|status]" >&2
        exit 1
        ;;
esac
