#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCAL_DENYLIST="${SINGULARI_PRIVACY_DENYLIST:-$ROOT_DIR/.privacy-denylist}"

cd "$ROOT_DIR"

DEFAULT_REGEX_PATTERNS=(
  "/Users/"
)

SAFE_REGEX_PLACEHOLDERS=(
  "/Users/<user>"
  "/Users/you"
)

FIXED_PATTERNS=()

add_fixed_pattern() {
  local value="$1"
  if [[ -n "$value" ]]; then
    FIXED_PATTERNS+=("$value")
  fi
}

if [[ -f "$LOCAL_DENYLIST" ]]; then
  while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line%%#*}"
    line="${line#"${line%%[![:space:]]*}"}"
    line="${line%"${line##*[![:space:]]}"}"
    add_fixed_pattern "$line"
  done <"$LOCAL_DENYLIST"
fi

if [[ -n "${SINGULARI_PRIVACY_DENYLIST_INLINE:-}" ]]; then
  while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line%%#*}"
    line="${line#"${line%%[![:space:]]*}"}"
    line="${line%"${line##*[![:space:]]}"}"
    add_fixed_pattern "$line"
  done <<<"$SINGULARI_PRIVACY_DENYLIST_INLINE"
fi

# Catch accidental local-author pushes without committing personal identifiers.
add_fixed_pattern "$(git config --global --get user.name || true)"
add_fixed_pattern "$(git config --global --get user.email || true)"

LOG_FILE="$(mktemp "${TMPDIR:-/tmp}/singulari-world-privacy-log.XXXXXX")"
PATH_FILE="$(mktemp "${TMPDIR:-/tmp}/singulari-world-privacy-paths.XXXXXX")"
trap 'rm -f "$LOG_FILE" "$PATH_FILE"' EXIT

git log --all --format='%H%n%an <%ae>%n%cn <%ce>%n%s%n%b' >"$LOG_FILE"
git ls-files >"$PATH_FILE"
HISTORY_REF="${SINGULARI_PRIVACY_HISTORY_REF:-HEAD}"

FAILED=0

report_failure() {
  local label="$1"
  echo "privacy-audit failed: $label" >&2
  FAILED=1
}

filter_safe_regex_placeholders() {
  local matches="$1"
  local safe
  for safe in "${SAFE_REGEX_PLACEHOLDERS[@]}"; do
    matches="$(printf '%s\n' "$matches" | grep -v -F -- "$safe" || true)"
  done
  printf '%s' "$matches"
}

report_regex_matches() {
  local label="$1"
  local pattern="$2"
  local matches="$3"
  matches="$(filter_safe_regex_placeholders "$matches")"
  if [[ -n "$matches" ]]; then
    printf '%s\n' "$matches"
    report_failure "$label matched regex: $pattern"
  fi
}

check_regex_pattern() {
  local pattern="$1"
  local matches

  matches="$(grep -n -E -- "$pattern" "$PATH_FILE" || true)"
  report_regex_matches "tracked path" "$pattern" "$matches"

  matches="$(git grep -I -n -E -- "$pattern" -- . ":(exclude)scripts/privacy-audit.sh" || true)"
  report_regex_matches "tracked content" "$pattern" "$matches"

  matches="$(grep -n -E -- "$pattern" "$LOG_FILE" || true)"
  report_regex_matches "git history" "$pattern" "$matches"

  while IFS= read -r rev; do
    matches="$(git grep -I -n -E -- "$pattern" "$rev" -- . ":(exclude)scripts/privacy-audit.sh" || true)"
    report_regex_matches "tracked historical content" "$pattern" "$matches"
  done < <(git rev-list "$HISTORY_REF")
}

check_fixed_pattern() {
  local pattern="$1"

  if grep -n -F -- "$pattern" "$PATH_FILE"; then
    report_failure "tracked path matched local pattern"
  fi

  if git grep -I -n -F -- "$pattern" -- . ":(exclude)scripts/privacy-audit.sh"; then
    report_failure "tracked content matched local pattern"
  fi

  if grep -n -F -- "$pattern" "$LOG_FILE"; then
    report_failure "git history matched local pattern"
  fi

  while IFS= read -r rev; do
    if git grep -I -n -F -- "$pattern" "$rev" -- .; then
      report_failure "tracked historical content matched local pattern"
    fi
  done < <(git rev-list "$HISTORY_REF")
}

for pattern in "${DEFAULT_REGEX_PATTERNS[@]}"; do
  check_regex_pattern "$pattern"
done

for pattern in "${FIXED_PATTERNS[@]}"; do
  check_fixed_pattern "$pattern"
done

if [[ "$FAILED" -ne 0 ]]; then
  exit 1
fi

echo "privacy audit ok"
