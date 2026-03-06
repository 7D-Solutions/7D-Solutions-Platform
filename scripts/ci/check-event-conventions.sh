#!/usr/bin/env bash
# scripts/ci/check-event-conventions.sh
#
# CI guardrail: enforce canonical EventEnvelope and event conventions.
#
# Checks:
#   1. No crate defines its own EventEnvelope struct outside platform/event-bus/.
#   2. No platform crate hard-codes .events. in NATS subject strings
#      (use nats_subject() from platform-contracts instead).
#   3. No schema_version values use path-style formats (/ or .json).
#
# All checks scope to production source code (src/ directories, not tests).
# Business modules (modules/*) are excluded from check 2 — they follow
# their own conventions and may hard-code subjects legitimately.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

errors=()

# ============================================================================
# Check 1: EventEnvelope fork detection
#
# Only platform/event-bus/ should define `struct EventEnvelope`.
# Test doubles in e2e-tests/ and test files are acceptable.
# ============================================================================
echo "--- Check 1: EventEnvelope fork detection"

while IFS= read -r line; do
    file="${line%%:*}"
    # Allow canonical location
    [[ "$file" == platform/event-bus/* ]] && continue
    # Allow test doubles
    [[ "$file" == e2e-tests/* ]] && continue
    [[ "$file" == */tests/* ]] && continue
    errors+=("ENVELOPE_FORK: $line")
done < <(grep -rn 'pub struct EventEnvelope\|pub(crate) struct EventEnvelope\|struct EventEnvelope' \
    --include='*.rs' platform/ modules/ tools/ 2>/dev/null || true)

if ! printf '%s\n' "${errors[@]}" 2>/dev/null | grep -q '^ENVELOPE_FORK:'; then
    echo "  OK"
fi

# ============================================================================
# Check 2: Hard-coded .events. subjects in platform crates
#
# Platform crates should use nats_subject() from platform-contracts to build
# NATS subjects, not hard-code the ".events." segment in string literals.
# This prevents divergence like identity-auth's former "auth.events.user.registered".
#
# Business modules (modules/*) are excluded — they may hard-code subjects
# and follow their own conventions within the {module}.events.{type} pattern.
#
# Only string literals (inside quotes) are checked to avoid false positives
# from struct field accesses like `req.events.is_empty()`.
# ============================================================================
echo "--- Check 2: Hard-coded .events. subjects in platform crates"

EVENTS_ALLOW=(
    "platform/event-bus/"               # canonical event bus implementation
    "platform/platform-contracts/"      # defines nats_subject() helper
    "platform/event-consumer/"          # consumer framework — documents subjects
    "platform/audit/"                   # event classification (reads subjects, not publishes)
)

while IFS= read -r line; do
    file="${line%%:*}"

    # Skip test files and bin utilities
    [[ "$file" == */tests/* ]] && continue
    [[ "$file" == */bin/* ]] && continue

    # Skip allowlisted paths
    skip=false
    for allow in "${EVENTS_ALLOW[@]}"; do
        if [[ "$file" == ${allow}* ]]; then
            skip=true
            break
        fi
    done
    [[ "$skip" == true ]] && continue

    # Skip comment lines
    content="${line#*:*:}"
    trimmed="${content#"${content%%[![:space:]]*}"}"
    [[ "$trimmed" == //* ]] && continue

    errors+=("HARDCODED_EVENTS_SUBJECT: $line")
done < <(grep -rn '"[^"]*\.events\.[^"]*"' --include='*.rs' \
    platform/*/src/ 2>/dev/null || true)

if ! printf '%s\n' "${errors[@]}" 2>/dev/null | grep -q '^HARDCODED_EVENTS_SUBJECT:'; then
    echo "  OK"
fi

# ============================================================================
# Check 3: Non-semver schema_version values
#
# schema_version must be a simple version string (e.g. "1", "1.0.0", "2").
# Flags values that contain "/" (path-style like "foo/v1") or end with
# ".json" (file-path style). Only checks production source code.
# ============================================================================
echo "--- Check 3: Non-semver schema_version formats"

# Detect schema_version values with "/" (path-style)
while IFS= read -r line; do
    file="${line%%:*}"
    [[ "$file" == */tests/* ]] && continue
    [[ "$file" == *event-bus/* ]] && continue

    # Skip comment lines
    content="${line#*:*:}"
    trimmed="${content#"${content%%[![:space:]]*}"}"
    [[ "$trimmed" == //* ]] && continue

    errors+=("SCHEMA_VERSION_SLASH: $line")
done < <(grep -rn 'schema_version.*"[^"]*\/[^"]*"\|with_schema_version.*"[^"]*\/[^"]*"' \
    --include='*.rs' platform/*/src/ modules/*/src/ 2>/dev/null || true)

# Detect schema_version values with ".json" (file-path style)
while IFS= read -r line; do
    file="${line%%:*}"
    [[ "$file" == */tests/* ]] && continue
    [[ "$file" == *event-bus/* ]] && continue

    content="${line#*:*:}"
    trimmed="${content#"${content%%[![:space:]]*}"}"
    [[ "$trimmed" == //* ]] && continue

    errors+=("SCHEMA_VERSION_JSON: $line")
done < <(grep -rn 'schema_version.*"[^"]*\.json[^"]*"\|with_schema_version.*"[^"]*\.json[^"]*"' \
    --include='*.rs' platform/*/src/ modules/*/src/ 2>/dev/null || true)

if ! printf '%s\n' "${errors[@]}" 2>/dev/null | grep -q '^SCHEMA_VERSION_'; then
    echo "  OK"
fi

# ============================================================================
# Report
# ============================================================================
echo ""
if [[ ${#errors[@]} -gt 0 ]]; then
    echo "ERROR: Event convention violations detected (${#errors[@]}):" >&2
    echo "" >&2
    for err in "${errors[@]}"; do
        echo "  $err" >&2
    done
    echo "" >&2
    echo "RULES:" >&2
    echo "  - EventEnvelope: only platform/event-bus/ may define struct EventEnvelope" >&2
    echo "  - Platform subjects: use nats_subject() from platform-contracts, not hard-coded .events." >&2
    echo "  - schema_version: use semver (\"1\", \"1.0.0\"), not paths (\"foo/v1\", \"foo.json\")" >&2
    echo "" >&2
    echo "See platform/platform-contracts/src/event_naming.rs for conventions." >&2
    exit 1
fi

echo "All event convention checks passed."
