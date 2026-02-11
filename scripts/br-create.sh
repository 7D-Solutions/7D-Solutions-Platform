#!/usr/bin/env bash
# br-create.sh - Bead creation with automatic work brief enrichment
#
# Wraps `br create` to automatically infer the bead type from keywords
# and append structured constraints from .agent-profiles/types.yaml.
#
# Usage:
#   ./scripts/br-create.sh "Fix login API endpoint"
#   ./scripts/br-create.sh "Update CSS layout" --type bug --parent bd-xxx
#   ./scripts/br-create.sh "Add tests" -d "Cover edge cases" --labels qa
#   ./scripts/br-create.sh "Add API route" --infer-type backend
#
# All flags except --infer-type are passed through to `br create`.
# The wrapper only enriches the --description with a WORK BRIEF section.
#
# Flags:
#   --infer-type <type>  Force a specific work brief type instead of keyword
#                        inference. Valid types: general, backend, frontend,
#                        devops, docs, qa
#
# Part of: Autonomous Agent Lifecycle System (bd-3u96)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TYPES_FILE="$PROJECT_ROOT/.agent-profiles/types.yaml"

# Source shared type inference
source "$SCRIPT_DIR/lib-infer-type.sh"

# Valid type names (must match types.yaml entries)
VALID_TYPES="general backend frontend devops docs qa"

#######################################
# Validate a type name against known types
# Arguments: $1 = type name
# Returns: 0 if valid, 1 if invalid
#######################################
validate_type() {
    local type_name="$1"
    for valid in $VALID_TYPES; do
        if [ "$type_name" = "$valid" ]; then
            return 0
        fi
    done
    return 1
}

#######################################
# Load constraints template for a type from types.yaml
# Arguments: $1 = type name
# Returns: template text (or empty string)
#######################################
load_template() {
    local type_name="$1"

    if [ ! -f "$TYPES_FILE" ]; then
        echo ""
        return
    fi

    # Use yq if available, fall back to simple extraction
    if command -v yq >/dev/null 2>&1; then
        local template
        template=$(yq eval ".agent_types[] | select(.name == \"$type_name\") | .constraints_template // \"\"" "$TYPES_FILE" 2>/dev/null)
        if [ -n "$template" ] && [ "$template" != "null" ] && [ "$template" != '""' ]; then
            echo "$template"
            return
        fi
    fi

    echo ""
}

#######################################
# Parse arguments to extract title and description
# We need to intercept -d/--description to enrich it
#######################################
main() {
    local title=""
    local description=""
    local labels=""
    local has_description=false
    local passthrough_args=()
    local forced_type=""

    # First pass: extract title, description, and labels for inference
    local skip_next=false
    for arg in "$@"; do
        if [ "$skip_next" = true ]; then
            skip_next=false
            continue
        fi

        case "$arg" in
            -d|--description)
                # Next arg is the description value
                has_description=true
                skip_next=true
                ;;
            -l|--labels)
                skip_next=true
                ;;
            --infer-type)
                skip_next=true
                ;;
            *)
                ;;
        esac
    done

    # Second pass: build the actual args
    local args=()
    skip_next=false
    for arg in "$@"; do
        if [ "$skip_next" = true ]; then
            case "${prev_flag}" in
                -d|--description)
                    description="$arg"
                    ;;
                -l|--labels)
                    labels="$arg"
                    args+=("$prev_flag" "$arg")
                    ;;
                --infer-type)
                    forced_type="$arg"
                    ;;
                *)
                    args+=("$prev_flag" "$arg")
                    ;;
            esac
            skip_next=false
            continue
        fi

        case "$arg" in
            -d|--description|-l|--labels|--infer-type)
                prev_flag="$arg"
                skip_next=true
                ;;
            *)
                # First non-flag arg without a preceding flag is the title
                if [ -z "$title" ] && [[ "$arg" != -* ]]; then
                    title="$arg"
                    args+=("$arg")
                else
                    args+=("$arg")
                fi
                ;;
        esac
    done

    if [ -z "$title" ]; then
        echo "Error: No title provided" >&2
        echo "Usage: $(basename "$0") \"Bead title\" [br create options...]" >&2
        exit 1
    fi

    # Use forced type or infer from title + description + labels
    local inferred_type
    if [ -n "$forced_type" ]; then
        if ! validate_type "$forced_type"; then
            echo "Error: Invalid --infer-type value: $forced_type" >&2
            echo "Valid types: $VALID_TYPES" >&2
            exit 1
        fi
        inferred_type="$forced_type"
    else
        inferred_type=$(infer_agent_type "$title" "$description" "$labels")
    fi

    # Load constraints template
    local template
    template=$(load_template "$inferred_type")

    # Build enriched description
    local enriched_description=""
    if [ -n "$description" ]; then
        enriched_description="$description"
    fi

    if [ -n "$template" ]; then
        if [ -n "$enriched_description" ]; then
            enriched_description="${enriched_description}

[WORK BRIEF]
${template}"
        else
            enriched_description="[WORK BRIEF]
${template}"
        fi
    fi

    # Write enriched description to temp file to avoid shell escaping issues
    local desc_file
    desc_file=$(mktemp /tmp/br-create-desc.XXXXXX)
    trap "rm -f '$desc_file'" EXIT

    echo "$enriched_description" > "$desc_file"

    # Build final br create command
    # Use --description= syntax to prevent content being parsed as flags
    if [ -n "$enriched_description" ]; then
        br create "${args[@]}" -d "$(cat "$desc_file")"
    else
        br create "${args[@]}"
    fi

    # Log what we did (stderr so it doesn't interfere with br output)
    echo "[br-create] Inferred type: $inferred_type" >&2

    # Notify all agents that a new bead is available
    if [ -x "$SCRIPT_DIR/broadcast-to-swarm.sh" ]; then
        "$SCRIPT_DIR/broadcast-to-swarm.sh" @active \
            "New bead available" \
            "New bead created: ${args[0]:-}. Run ./scripts/bv-claim.sh to claim it." \
            --type FYI --mail-only >/dev/null 2>&1 &
    fi
}

main "$@"
