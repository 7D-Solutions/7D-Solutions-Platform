#!/usr/bin/env bash
# scripts/ci/check-openapi-breaking-changes.sh
#
# Detect breaking changes in OpenAPI YAML/JSON specs under contracts/.
# Fails CI if breaking changes are introduced without both a MAJOR version
# bump in info.version AND (in PR context) a BREAKING-CHANGE: annotation in
# the PR description.
#
# Breaking changes detected:
#   - Removed API paths (endpoints deleted)
#   - Required request/response schema fields removed
#
# Bypass (intentional breaking changes):
#   1. Bump info.version in the modified spec to the next MAJOR version
#      (e.g. 1.3.2 → 2.0.0, or 0.1.0 → 1.0.0)
#   2. Add to the PR description:
#        BREAKING-CHANGE: <one-line migration note>
#      e.g. "BREAKING-CHANGE: removed deprecated /v1/invoice — use /v2/invoice"
#   Both conditions must be met on a PR. For local / push-to-main runs, only
#   the MAJOR version bump is checked (PR description is unavailable).
#
# Usage:
#   check-openapi-breaking-changes.sh [BASE_REF]
#
# In GitHub Actions the base ref is auto-detected via GITHUB_BASE_REF (PR) or
# GITHUB_EVENT_BEFORE (push to main).
# Pass the PR body via:   env PR_BODY="${{ github.event.pull_request.body }}"

set -euo pipefail

# ---------------------------------------------------------------------------
# Determine base ref
# ---------------------------------------------------------------------------
if [[ -n "${GITHUB_BASE_REF:-}" ]]; then
    # Pull-request context — GITHUB_BASE_REF is the target branch name (e.g. "main")
    git fetch --quiet origin "${GITHUB_BASE_REF}" 2>/dev/null || true
    BASE="origin/${GITHUB_BASE_REF}"
elif [[ -n "${GITHUB_EVENT_BEFORE:-}" && \
        "${GITHUB_EVENT_BEFORE}" != "0000000000000000000000000000000000000000" ]]; then
    # Push-to-main context — compare against the previous tip
    BASE="${GITHUB_EVENT_BEFORE}"
else
    # Local / fallback
    BASE="${1:-HEAD~1}"
fi

echo "🔍 Contract breaking-change gate"
echo "   Base ref: ${BASE}"
echo ""

# ---------------------------------------------------------------------------
# Extract BREAKING-CHANGE note from PR body (set via env PR_BODY in CI)
# ---------------------------------------------------------------------------
BREAKING_CHANGE_NOTE=""
if [[ -n "${PR_BODY:-}" ]]; then
    BREAKING_CHANGE_NOTE=$(printf '%s\n' "${PR_BODY}" \
        | grep -oE 'BREAKING-CHANGE:[[:space:]]*[^[:space:]].*' \
        | head -1 \
        | sed 's/^BREAKING-CHANGE:[[:space:]]*//' \
        | tr -d '\r' \
        || true)
fi
export BREAKING_CHANGE_NOTE
# IN_PR_CONTEXT is truthy when PR_BODY was provided (even if BREAKING_CHANGE_NOTE is empty)
IN_PR_CONTEXT="${PR_BODY:+1}"
export IN_PR_CONTEXT

if [[ -n "${IN_PR_CONTEXT}" ]]; then
    if [[ -n "${BREAKING_CHANGE_NOTE}" ]]; then
        echo "   PR breaking-change note: \"${BREAKING_CHANGE_NOTE}\""
    else
        echo "   PR context detected — BREAKING-CHANGE: annotation not found in PR description"
    fi
fi
echo ""

# ---------------------------------------------------------------------------
# Find changed OpenAPI YAML and JSON files in contracts/
# ---------------------------------------------------------------------------
changed_specs=$(git diff --name-only "${BASE}...HEAD" \
        -- 'contracts/**/*.yaml' 'contracts/**/*.json' 2>/dev/null \
    || git diff --name-only "${BASE}" HEAD \
        -- 'contracts/**/*.yaml' 'contracts/**/*.json' 2>/dev/null \
    || true)

if [[ -z "${changed_specs}" ]]; then
    echo "✓ No OpenAPI spec changes detected — gate passes"
    exit 0
fi

echo "Changed specs:"
echo "${changed_specs}" | sed 's/^/  /'
echo ""

# ---------------------------------------------------------------------------
# Python analyser — written to a temp file to avoid quoting issues
# ---------------------------------------------------------------------------
# Ensure pyyaml is available (pre-installed on ubuntu-latest; auto-install locally)
if ! python3 -c "import yaml" 2>/dev/null; then
    echo "Installing pyyaml..."
    python3 -m pip install --quiet pyyaml 2>/dev/null \
        || python3 -m pip install --quiet --break-system-packages pyyaml 2>/dev/null \
        || python3 -m pip install --quiet --user pyyaml 2>/dev/null \
        || { echo "ERROR: pyyaml not available and could not be installed" >&2; exit 1; }
fi

SCRIPT=$(mktemp /tmp/check_openapi_XXXXXX.py)
trap 'rm -f "$SCRIPT"' EXIT

cat > "$SCRIPT" <<'PYEOF'
#!/usr/bin/env python3
"""
Detect breaking changes in OpenAPI specs.
Breaking = removed paths or removed required fields.

Bypass (both required on PRs; MAJOR bump only for local/push-to-main):
  1. info.version bumped to a higher MAJOR (e.g. 1.3.2 → 2.0.0)
  2. PR description contains: BREAKING-CHANGE: <migration note>
"""
import os
import sys
import subprocess
import yaml  # pyyaml


def git_show(ref: str, filepath: str):
    """Return parsed YAML/JSON from a git ref, or None if the file doesn't exist there."""
    try:
        result = subprocess.run(
            ["git", "show", f"{ref}:{filepath}"],
            capture_output=True, text=True, check=True,
        )
        return yaml.safe_load(result.stdout)
    except subprocess.CalledProcessError:
        return None


def info_version(spec) -> str:
    if not spec:
        return "0.0.0"
    ver = spec.get("info", {}).get("version", "0.0.0")
    return str(ver) if ver is not None else "0.0.0"


def parse_semver(version_str: str):
    """Parse X.Y.Z (handles v-prefix and 2-part versions). Returns (major, minor, patch) or None."""
    if not version_str:
        return None
    s = str(version_str).lstrip("v").strip()
    parts = s.split(".")
    try:
        major = int(parts[0]) if len(parts) > 0 else 0
        minor = int(parts[1]) if len(parts) > 1 else 0
        patch = int(parts[2].split("-")[0]) if len(parts) > 2 else 0
        return (major, minor, patch)
    except (ValueError, IndexError):
        return None


def is_major_bump(old_ver_str: str, new_ver_str: str) -> bool:
    """Return True if new_ver has a strictly higher major component than old_ver."""
    old = parse_semver(old_ver_str)
    new = parse_semver(new_ver_str)
    if old is None or new is None:
        # Unparseable — treat any change as sufficient
        return old_ver_str != new_ver_str
    return new[0] > old[0]


def check_acknowledgement(old_ver: str, new_ver: str) -> tuple:
    """
    Returns (acknowledged: bool, reason: str).

    Rules:
      - info.version must be bumped to a higher MAJOR (e.g. 1.x.x → 2.0.0)
      - In PR context (PR_BODY env set): PR description must also contain
        BREAKING-CHANGE: <note>
    """
    in_pr = bool(os.environ.get("IN_PR_CONTEXT", "").strip())
    breaking_note = os.environ.get("BREAKING_CHANGE_NOTE", "").strip()

    major_bumped = is_major_bump(old_ver, new_ver)
    ver_changed = old_ver != new_ver

    if not ver_changed:
        return False, "info.version not bumped (MAJOR bump required)"
    if not major_bumped:
        return False, (
            f"version bump {old_ver} → {new_ver} is not a MAJOR bump "
            f"(increment the leftmost non-zero segment to acknowledge breaking change)"
        )
    if in_pr and not breaking_note:
        return False, (
            "PR description is missing the required BREAKING-CHANGE: annotation\n"
            "  Add to your PR description:\n"
            "    BREAKING-CHANGE: <one-line migration note>\n"
            "  e.g. BREAKING-CHANGE: removed /v1/invoice — callers must use /v2/invoice"
        )

    return True, ""


def path_keys(spec) -> set:
    if not spec:
        return set()
    return set((spec.get("paths") or {}).keys())


def required_fields(spec, path: str, method: str, location: str) -> set:
    """Extract required field names from request or response schemas."""
    if not spec:
        return set()
    operation = (spec.get("paths") or {}).get(path, {}).get(method, {})
    fields = set()
    if location == "request":
        content = operation.get("requestBody", {}).get("content", {})
        for media in content.values():
            fields.update(media.get("schema", {}).get("required") or [])
    elif location == "response":
        for _status, resp in operation.get("responses", {}).items():
            for media in resp.get("content", {}).values():
                fields.update(media.get("schema", {}).get("required") or [])
    return fields


def analyse(base_ref: str, filepaths: list) -> list:
    """Returns list of (filepath, old_version, reason, [breaking_items]) for unacknowledged breakages."""
    unacknowledged = []

    for filepath in filepaths:
        old_spec = git_show(base_ref, filepath)
        if old_spec is None:
            print(f"  ✓ {filepath}: new spec — no base to compare against")
            continue

        try:
            with open(filepath) as f:
                new_spec = yaml.safe_load(f)
        except Exception as exc:
            print(f"  ✗ {filepath}: parse error — {exc}", file=sys.stderr)
            sys.exit(1)

        old_ver = info_version(old_spec)
        new_ver = info_version(new_spec)

        old_paths = path_keys(old_spec)
        new_paths = path_keys(new_spec)
        removed_paths = old_paths - new_paths

        breaking = []

        # Removed endpoints
        for rp in sorted(removed_paths):
            breaking.append(f"  REMOVED ENDPOINT: {rp}")

        # Removed required request fields on surviving paths
        for path in sorted(old_paths & new_paths):
            for method in ["get", "post", "put", "patch", "delete"]:
                old_req = required_fields(old_spec, path, method, "request")
                new_req = required_fields(new_spec, path, method, "request")
                for field in sorted(old_req - new_req):
                    breaking.append(
                        f"  REMOVED REQUIRED FIELD: {method.upper()} {path} → body.{field}"
                    )

        if breaking:
            print(f"\n📋 {filepath}")
            print(f"   Version: {old_ver} → {new_ver}")
            for item in breaking:
                print(item)

            acknowledged, reason = check_acknowledgement(old_ver, new_ver)
            if acknowledged:
                breaking_note = os.environ.get("BREAKING_CHANGE_NOTE", "").strip()
                note_display = f" (\"{breaking_note}\")" if breaking_note else ""
                print(f"   ✓ Acknowledged — MAJOR version bump + PR annotation{note_display}")
            else:
                unacknowledged.append((filepath, old_ver, new_ver, reason, breaking))
        else:
            print(f"  ✓ {filepath}: no breaking changes "
                  f"(version {old_ver} → {new_ver})")

    return unacknowledged


def main():
    if len(sys.argv) < 2:
        print("Usage: check_openapi.py BASE_REF [file ...]", file=sys.stderr)
        sys.exit(1)

    base_ref = sys.argv[1]
    filepaths = sys.argv[2:]

    unacknowledged = analyse(base_ref, filepaths)

    if unacknowledged:
        print("\n" + "=" * 70, file=sys.stderr)
        print("❌  CONTRACT BREAKING-CHANGE GATE FAILED", file=sys.stderr)
        print("=" * 70, file=sys.stderr)
        print("", file=sys.stderr)
        for filepath, old_ver, new_ver, reason, items in unacknowledged:
            print(f"  Spec   : {filepath}", file=sys.stderr)
            print(f"  Version: {old_ver} → {new_ver}", file=sys.stderr)
            print(f"  Reason : {reason}", file=sys.stderr)
            print(f"  Changes:", file=sys.stderr)
            for item in items:
                print(f"    {item.strip()}", file=sys.stderr)
            print("", file=sys.stderr)
        print("=" * 70, file=sys.stderr)
        print("To bypass this gate (intentional breaking change):", file=sys.stderr)
        print("  1. Bump info.version in the spec to the next MAJOR version", file=sys.stderr)
        print("       example: 1.3.2 → 2.0.0   or   0.1.0 → 1.0.0", file=sys.stderr)
        print("  2. Add to your PR description:", file=sys.stderr)
        print("       BREAKING-CHANGE: <one-line migration note>", file=sys.stderr)
        print("       example: BREAKING-CHANGE: removed /v1/invoice — use /v2/invoice", file=sys.stderr)
        print("", file=sys.stderr)
        print("Both conditions must be present. PATCH/MINOR bumps are NOT sufficient.", file=sys.stderr)
        print("See docs/architecture/CONTRACT-VERSIONING-POLICY.md", file=sys.stderr)
        sys.exit(1)

    print("\n✓ Contract breaking-change gate: PASS")


if __name__ == "__main__":
    main()
PYEOF

# shellcheck disable=SC2086
python3 "$SCRIPT" "${BASE}" ${changed_specs}
