#!/usr/bin/env bash
# scripts/ci/check-openapi-breaking-changes.sh
#
# Detect breaking changes in OpenAPI YAML specs under contracts/.
# Fails CI if breaking changes are introduced without a version bump in info.version.
#
# Breaking changes detected:
#   - Removed API paths (endpoints deleted)
#   - Required request/response schema fields removed
#
# Acknowledgment mechanism:
#   Bump info.version in the modified spec file.
#   PATCH (0.0.x) or higher is sufficient — any bump acknowledges the intent.
#
# Usage:
#   check-openapi-breaking-changes.sh [BASE_REF]
#
# In GitHub Actions the base ref is auto-detected via GITHUB_BASE_REF (PR) or
# GITHUB_EVENT_BEFORE (push to main).

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
# Find changed OpenAPI YAML files in contracts/
# ---------------------------------------------------------------------------
changed_specs=$(git diff --name-only "${BASE}...HEAD" -- 'contracts/**/*.yaml' 2>/dev/null \
    || git diff --name-only "${BASE}" HEAD -- 'contracts/**/*.yaml' 2>/dev/null \
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
Acknowledged = info.version changed between base and head.
"""
import sys
import subprocess
import yaml  # pyyaml


def git_show(ref: str, filepath: str):
    """Return parsed YAML from a git ref, or None if the file doesn't exist there."""
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
    return spec.get("info", {}).get("version", "0.0.0")


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


def analyse(base_ref: str, filepaths: list[str]) -> list[tuple]:
    """Returns list of (filepath, old_version, [breaking_items]) for unacknowledged breakages."""
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
        bumped = old_ver != new_ver

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
            print(f"   Version: {old_ver} → {new_ver} "
                  f"{'(bumped ✓)' if bumped else '(NOT bumped ✗)'}")
            for item in breaking:
                print(item)
            if bumped:
                print(f"   ✓ Breaking changes acknowledged via version bump")
            else:
                unacknowledged.append((filepath, old_ver, breaking))
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
        for filepath, version, items in unacknowledged:
            print(f"  Spec  : {filepath}", file=sys.stderr)
            print(f"  Version: {version} (unchanged)", file=sys.stderr)
            print(f"  Changes:", file=sys.stderr)
            for item in items:
                print(f"    {item.strip()}", file=sys.stderr)
            print("", file=sys.stderr)
        print("To acknowledge breaking changes, bump info.version in the spec.", file=sys.stderr)
        print("  PATCH bump: sufficient for any acknowledged breaking change", file=sys.stderr)
        print("  MAJOR bump: required for proven modules (v1.x.x+) per VERSIONING.md", file=sys.stderr)
        print("", file=sys.stderr)
        print("See docs/architecture/CONTRACT-VERSIONING-POLICY.md", file=sys.stderr)
        sys.exit(1)

    print("\n✓ Contract breaking-change gate: PASS")


if __name__ == "__main__":
    main()
PYEOF

# shellcheck disable=SC2086
python3 "$SCRIPT" "${BASE}" ${changed_specs}
