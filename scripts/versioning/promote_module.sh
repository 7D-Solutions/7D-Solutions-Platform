#!/usr/bin/env bash
# promote_module.sh — Single-command module promotion respecting Gates 1, 2, and 3.
#
# Pipeline order:
#   1. Pre-flight — proof script exists, version sane, tag unique, clean worktree
#   2. REVISIONS.md — generates stub if missing; exits if TODOs remain
#   3. Proof script — build + tests must pass (Gate 1)
#   4. REVISIONS lint — all fields complete, no placeholders
#   5. Version bump in package file (Cargo.toml or package.json)
#   6. Update staging MODULE-MANIFEST.md
#   7. Commit version bump + manifest update
#   8. Create git tag {module-name}-v{version}
#   9. Output next action (or --push-tag: push tag to trigger Gate 2 CI)
#  10. (Optional) Run staging proof gate — if --staging-host is set
#
# Usage:
#   bash scripts/versioning/promote_module.sh \
#     --module modules/ar --version 1.0.0 --bead bd-qvbg
#
#   bash scripts/versioning/promote_module.sh \
#     --module modules/ar --bump-type patch --bead bd-qvbg \
#     --push-tag --staging-host staging.example.com
#
# Options:
#   --module <path>                  Module dir (e.g. modules/ar, platform/identity-auth)
#   --version <semver>               Target version — mutually exclusive with --bump-type
#   --bump-type <patch|minor|major>  Compute next version from current
#   --bead <id>                      Bead ID for commit message and REVISIONS.md
#   --push-tag                       Push the created tag (triggers Gate 2 CI)
#   --staging-host <host>            Run proof gate after deploy (optional)
#   --dry-run                        Print actions without making changes
#   --help                           Show this message
#
# Guards — refuses if:
#   - New version is "latest"
#   - Git tag {module}-v{version} already exists (overwrite protection)
#   - Proof script scripts/proof_{module}.sh does not exist
#   - Working tree has uncommitted changes
#   - REVISIONS.md entry has unfilled TODO placeholders
#
# Exit codes: 0 = success, 1 = failure or guard triggered

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

STAGING_MANIFEST="${REPO_ROOT}/deploy/staging/MODULE-MANIFEST.md"
IMAGE_REGISTRY="${IMAGE_REGISTRY:-7dsolutions}"

MODULE_PATH=""
TARGET_VERSION=""
BUMP_TYPE=""
BEAD_ID=""
PUSH_TAG=false
STAGING_HOST=""
DRY_RUN=false

usage() { awk 'NR==1{next} /^[^#]/{exit} {sub(/^# ?/,""); print}' "$0"; exit 0; }
die()  { echo "ERROR: $*" >&2; exit 1; }
step() { echo ""; echo "▶ $*"; }
ok()   { echo "  ✓ $*"; }
info() { echo "  · $*"; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --module)        MODULE_PATH="${2:-}";    shift 2 ;;
        --version)       TARGET_VERSION="${2:-}"; shift 2 ;;
        --bump-type)     BUMP_TYPE="${2:-}";      shift 2 ;;
        --bead)          BEAD_ID="${2:-}";        shift 2 ;;
        --push-tag)      PUSH_TAG=true;           shift ;;
        --staging-host)  STAGING_HOST="${2:-}";   shift 2 ;;
        --dry-run)       DRY_RUN=true;            shift ;;
        -h|--help)       usage ;;
        *) die "Unknown option: $1" ;;
    esac
done

[[ -z "$MODULE_PATH" ]] && die "--module is required"
[[ -z "$BEAD_ID" ]]     && die "--bead is required"
[[ -n "$TARGET_VERSION" && -n "$BUMP_TYPE" ]] && die "--version and --bump-type are mutually exclusive"
[[ -z "$TARGET_VERSION" && -z "$BUMP_TYPE" ]] && die "One of --version or --bump-type is required"

MODULE_ABS="${REPO_ROOT}/${MODULE_PATH}"
MODULE_NAME="$(basename "$MODULE_PATH")"
[[ -d "$MODULE_ABS" ]] || die "Module directory not found: $MODULE_PATH"

PACKAGE_FILE="" PACKAGE_TYPE=""
if [[ -f "${MODULE_ABS}/Cargo.toml" ]]; then
    PACKAGE_FILE="${MODULE_ABS}/Cargo.toml"; PACKAGE_TYPE="cargo"
elif [[ -f "${MODULE_ABS}/package.json" ]]; then
    PACKAGE_FILE="${MODULE_ABS}/package.json"; PACKAGE_TYPE="npm"
else
    die "No Cargo.toml or package.json found in $MODULE_PATH"
fi

read_current_version() {
    if [[ "$PACKAGE_TYPE" == "cargo" ]]; then
        awk '/^\[package\]/{p=1;next}/^\[/{p=0}p&&/^version[[:space:]]*=/{print;exit}' \
            "$PACKAGE_FILE" | sed 's/.*"\([^"]*\)".*/\1/'
    else
        python3 -c "import json; print(json.load(open('$PACKAGE_FILE'))['version'])"
    fi
}

CURRENT_VERSION="$(read_current_version)"
[[ -z "$CURRENT_VERSION" ]] && die "Could not read version from $PACKAGE_FILE"

bump_version() {
    local ver="$1" type="$2"
    local major minor patch
    IFS='.' read -r major minor patch <<< "$ver"
    case "$type" in
        major) echo "$((major + 1)).0.0" ;;
        minor) echo "${major}.$((minor + 1)).0" ;;
        patch) echo "${major}.${minor}.$((patch + 1))" ;;
        *) die "Invalid bump type: $type (must be patch, minor, or major)" ;;
    esac
}

[[ -n "$BUMP_TYPE" ]] && TARGET_VERSION="$(bump_version "$CURRENT_VERSION" "$BUMP_TYPE")"
echo "$TARGET_VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$' \
    || die "Version must be SemVer (e.g. 1.0.0): '$TARGET_VERSION'"

GIT_TAG="${MODULE_NAME}-v${TARGET_VERSION}"

echo "========================================================"
echo "  promote_module.sh"
printf "  Module:  %s\n  Version: %s → %s\n  Tag:     %s\n  Bead:    %s\n" \
    "$MODULE_PATH" "$CURRENT_VERSION" "$TARGET_VERSION" "$GIT_TAG" "$BEAD_ID"
[[ "$DRY_RUN" == "true" ]] && echo "  Mode:    DRY-RUN"
echo "========================================================"

# Step 1 — Pre-flight guards
step "Pre-flight checks"

[[ "$TARGET_VERSION" == "latest" || "$TARGET_VERSION" == *":latest" ]] \
    && die "Cannot promote to 'latest'. Specify an immutable SemVer tag."
ok "Version is not 'latest'"

if git tag -l "$GIT_TAG" | grep -q "$GIT_TAG"; then
    die "Tag '$GIT_TAG' already exists. Bump the version instead of overwriting."
fi
ok "Tag '$GIT_TAG' does not exist"

PROOF_SCRIPT_NAME="proof_${MODULE_NAME//-/_}.sh"
PROOF_SCRIPT="${REPO_ROOT}/scripts/${PROOF_SCRIPT_NAME}"
[[ -f "$PROOF_SCRIPT" ]] || die "Proof script not found: scripts/${PROOF_SCRIPT_NAME}\n       See docs/VERSIONING.md § 'Proof command requirement'."
ok "Proof script exists: scripts/${PROOF_SCRIPT_NAME}"

git diff --quiet HEAD 2>/dev/null \
    || die "Working tree has uncommitted changes. Commit or stash them first."
ok "Working tree is clean"

# Step 2 — REVISIONS.md check
step "REVISIONS.md check (v${TARGET_VERSION})"

REVISIONS_FILE="${MODULE_ABS}/REVISIONS.md"
TARGET_MAJOR="$(echo "$TARGET_VERSION" | cut -d. -f1)"

if [[ "$TARGET_MAJOR" -ge 1 ]]; then
    ESCAPED_VER="$(echo "$TARGET_VERSION" | sed 's/\./\\./g')"
    if [[ ! -f "$REVISIONS_FILE" ]] || ! grep -qE "^\| $ESCAPED_VER[[:space:]]*\|" "$REVISIONS_FILE"; then
        info "No entry for v${TARGET_VERSION} — generating stub..."
        if [[ "$DRY_RUN" == "false" ]]; then
            bash "${REPO_ROOT}/scripts/versioning/new_revision_entry.sh" \
                "$MODULE_PATH" "$TARGET_VERSION" "$BEAD_ID"
        else
            info "[dry-run] Would run: scripts/versioning/new_revision_entry.sh $MODULE_PATH $TARGET_VERSION $BEAD_ID"
        fi
        echo ""
        echo "ACTION REQUIRED: Fill in TODO fields in ${REVISIONS_FILE}, then re-run:"
        echo "  bash scripts/versioning/promote_module.sh \\"
        echo "    --module $MODULE_PATH --version $TARGET_VERSION --bead $BEAD_ID"
        exit 1
    fi
    ROW="$(grep -E "^\| $ESCAPED_VER[[:space:]]*\|" "$REVISIONS_FILE" | head -1)"
    if echo "$ROW" | grep -qi "TODO\|bd-xxxx\|YYYY-MM-DD"; then
        echo "STOP: REVISIONS.md v${TARGET_VERSION} still has placeholder values."
        echo "  Row: $ROW"
        echo "  Fill in all TODO fields in ${REVISIONS_FILE} and re-run."
        exit 1
    fi
    ok "REVISIONS.md entry for v${TARGET_VERSION} is complete"
else
    info "Unproven target (v${TARGET_VERSION} < 1.0.0) — REVISIONS.md not required"
fi

# Step 3 — Run proof script
step "Running proof: scripts/${PROOF_SCRIPT_NAME}"
if [[ "$DRY_RUN" == "false" ]]; then
    bash "$PROOF_SCRIPT" || die "Proof failed. Fix failures and re-run."
    ok "Proof passed"
else
    info "[dry-run] Would run: bash scripts/${PROOF_SCRIPT_NAME}"
fi

# Step 4 — REVISIONS lint
if [[ "$TARGET_MAJOR" -ge 1 ]]; then
    step "REVISIONS lint"
    if [[ "$DRY_RUN" == "false" ]]; then
        bash "${REPO_ROOT}/scripts/versioning/lint_revisions.sh" --module "$MODULE_PATH" \
            || die "REVISIONS lint failed. Fix errors and re-run."
        ok "REVISIONS lint passed"
    else
        info "[dry-run] Would run: scripts/versioning/lint_revisions.sh --module $MODULE_PATH"
    fi
fi

# Step 5 — Bump version in package file
step "Bumping version: ${CURRENT_VERSION} → ${TARGET_VERSION}"

update_package_version() {
    local new_ver="$1"
    if [[ "$PACKAGE_TYPE" == "cargo" ]]; then
        python3 - "$PACKAGE_FILE" "$new_ver" <<'PYEOF'
import sys, re
path, new_ver = sys.argv[1], sys.argv[2]
with open(path) as f:
    lines = f.readlines()
in_pkg = False; replaced = False; result = []
for line in lines:
    if re.match(r'^\[package\]', line): in_pkg = True
    elif re.match(r'^\[', line): in_pkg = False
    if in_pkg and not replaced and re.match(r'^version\s*=', line):
        line = re.sub(r'"[^"]*"', f'"{new_ver}"', line, count=1); replaced = True
    result.append(line)
if not replaced:
    print("ERROR: version field not found in [package] section", file=sys.stderr); sys.exit(1)
open(path, 'w').writelines(result)
PYEOF
    else
        python3 - "$PACKAGE_FILE" "$new_ver" <<'PYEOF'
import sys, json
path, new_ver = sys.argv[1], sys.argv[2]
with open(path) as f: data = json.load(f)
data['version'] = new_ver
with open(path, 'w') as f: json.dump(data, f, indent=2); f.write('\n')
PYEOF
    fi
}

if [[ "$DRY_RUN" == "false" ]]; then
    update_package_version "$TARGET_VERSION"
    VERIFIED="$(read_current_version)"
    [[ "$VERIFIED" == "$TARGET_VERSION" ]] \
        || die "Version mismatch after update: got '$VERIFIED', expected '$TARGET_VERSION'"
    ok "Version updated to $TARGET_VERSION in $PACKAGE_FILE"
else
    info "[dry-run] Would update: $PACKAGE_FILE version → $TARGET_VERSION"
fi

# Step 6 — Update staging manifest
step "Updating staging manifest"

GIT_SHA="$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")"
FULL_TAG="${IMAGE_REGISTRY}/${MODULE_NAME}:${TARGET_VERSION}-${GIT_SHA}"

update_manifest() {
    local manifest="$1" canonical="$2" new_ver="$3" sha="$4" full_tag="$5" bead="$6"
    python3 - "$manifest" "$canonical" "$new_ver" "$sha" "$full_tag" "$bead" <<'PYEOF'
import sys, re
manifest_path, canonical, new_ver, sha, full_tag, bead = sys.argv[1:]
with open(manifest_path) as f: lines = f.readlines()
found = False; result = []
for line in lines:
    if re.search(r'\|[^|]*`' + re.escape(canonical) + r'`[^|]*\|', line):
        parts = line.split('|')
        if len(parts) >= 7:
            parts[3] = f' {new_ver} '
            parts[4] = f' {sha} '
            parts[5] = f' `{full_tag}` '
            parts[6] = f' Promoted by {bead} '
            line = '|'.join(parts)
            if not line.endswith('\n'): line += '\n'
            found = True
    result.append(line)
if not found:
    print(f"WARNING: No manifest row for '{canonical}'. Add module to MODULE-MANIFEST.md first.", file=sys.stderr)
    sys.exit(1)
open(manifest_path, 'w').writelines(result)
PYEOF
}

if [[ "$DRY_RUN" == "false" ]]; then
    if [[ -f "$STAGING_MANIFEST" ]]; then
        update_manifest "$STAGING_MANIFEST" "$MODULE_NAME" "$TARGET_VERSION" \
                        "$GIT_SHA" "$FULL_TAG" "$BEAD_ID" \
            || info "WARN: Module not in manifest — skipping (add it to MODULE-MANIFEST.md first)"
        ok "Manifest updated: ${MODULE_NAME} → ${FULL_TAG}"
    else
        info "WARN: $STAGING_MANIFEST not found — skipping"
    fi
else
    info "[dry-run] Would update manifest: ${MODULE_NAME} → ${FULL_TAG}"
fi

# Step 7 — Commit
step "Creating promotion commit"
COMMIT_MSG="[${BEAD_ID}] ${MODULE_NAME} v${CURRENT_VERSION} → v${TARGET_VERSION}: promote"
if [[ "$DRY_RUN" == "false" ]]; then
    git add "$PACKAGE_FILE"
    [[ -f "$STAGING_MANIFEST" ]] && git add "$STAGING_MANIFEST"
    git commit -m "$COMMIT_MSG"
    ok "Committed: $COMMIT_MSG"
else
    info "[dry-run] Would commit: $COMMIT_MSG"
fi

# Step 8 — Create git tag
step "Creating git tag: ${GIT_TAG}"
if [[ "$DRY_RUN" == "false" ]]; then
    git tag "$GIT_TAG"
    ok "Tag created: $GIT_TAG"
else
    info "[dry-run] Would create tag: $GIT_TAG"
fi

# Step 9 — Optional: push tag (triggers Gate 2 CI)
if [[ "$PUSH_TAG" == "true" ]]; then
    step "Pushing tag to origin (triggers Gate 2 CI)"
    if [[ "$DRY_RUN" == "false" ]]; then
        git push origin "$GIT_TAG"
        ok "Pushed: $GIT_TAG → origin"
    else
        info "[dry-run] Would run: git push origin $GIT_TAG"
    fi
fi

# Step 10 — Optional: staging proof gate
if [[ -n "$STAGING_HOST" ]]; then
    step "Running staging proof gate against: ${STAGING_HOST}"
    if [[ "$DRY_RUN" == "false" ]]; then
        STAGING_HOST="$STAGING_HOST" bash "${REPO_ROOT}/scripts/staging/proof_gate.sh" \
            || die "Staging proof gate FAILED. Check /tmp/proof_gate_logs for details."
        ok "Staging proof gate PASSED"
    else
        info "[dry-run] Would run: scripts/staging/proof_gate.sh --host $STAGING_HOST"
    fi
fi

# Summary
echo ""
echo "========================================================"
echo "  Promotion complete: ${MODULE_NAME} v${TARGET_VERSION}"
echo "========================================================"
if [[ "$PUSH_TAG" == "false" ]]; then
    echo ""
    echo "  Next: push tag to trigger Gate 2 (CI builds the image):"
    echo "    git push origin ${GIT_TAG}"
    echo ""
    echo "  Then deploy via Gate 3 (promote.yml):"
    echo "    gh workflow run promote.yml --field tag=${MODULE_NAME}:${TARGET_VERSION}-<sha>"
else
    echo "  Tag pushed. Gate 2 CI is building the image."
    echo "  After CI: gh workflow run promote.yml --field tag=${MODULE_NAME}:${TARGET_VERSION}-<sha>"
fi
[[ "$DRY_RUN" == "true" ]] && echo "" && echo "  (DRY-RUN: no changes were made)"

exit 0
