#!/usr/bin/env bash
# lib-infer-type.sh - Shared type inference function
#
# Source this from other scripts:
#   source "$SCRIPT_DIR/lib-infer-type.sh"
#   type=$(infer_agent_type "$title" "$description" "$labels")
#
# Returns: agent type name (general, backend, frontend, devops, docs, qa)

#######################################
# Infer agent type from bead text
# Arguments: $1 = title, $2 = description, $3 = labels
# Returns: agent type name
#######################################
infer_agent_type() {
    local title="${1,,}"       # lowercase
    local description="${2,,}"
    local labels="${3,,}"
    local text="$title $description $labels"

    # Check labels first (most reliable)
    if [[ "$labels" == *"frontend"* ]] || [[ "$labels" == *"ui"* ]]; then
        echo "frontend"
        return
    fi
    if [[ "$labels" == *"backend"* ]] || [[ "$labels" == *"api"* ]]; then
        echo "backend"
        return
    fi
    if [[ "$labels" == *"devops"* ]] || [[ "$labels" == *"infrastructure"* ]]; then
        echo "devops"
        return
    fi
    if [[ "$labels" == *"docs"* ]] || [[ "$labels" == *"documentation"* ]]; then
        echo "docs"
        return
    fi
    if [[ "$labels" == *"qa"* ]] || [[ "$labels" == *"testing"* ]]; then
        echo "qa"
        return
    fi

    # Keyword inference from title/description
    # Order matters: check more specific patterns (QA, docs) before broad ones (backend)

    # QA patterns (check early — "test" is a strong signal)
    # Avoid overly broad terms: "validation" (appears in backend), "spec" (appears in docs)
    if [[ "$text" =~ (test|coverage|qa|lint|benchmark|e2e) ]]; then
        echo "qa"
        return
    fi

    # Docs patterns (check early — "document" is a strong signal)
    if [[ "$text" =~ (document|readme|guide|tutorial|changelog|write.up|specification|openapi) ]]; then
        echo "docs"
        return
    fi

    # DevOps patterns
    if [[ "$text" =~ (docker|kubernetes|ci.cd|deploy|pipeline|monitor|infrastructure|nginx|terraform|helm) ]]; then
        echo "devops"
        return
    fi

    # Frontend patterns
    if [[ "$text" =~ (css|component|ui|ux|react|vue|angular|layout|style|responsive|button|form|page|frontend) ]]; then
        echo "frontend"
        return
    fi

    # Backend patterns (last — broad terms like "api", "auth" are catch-all)
    if [[ "$text" =~ (api|database|endpoint|server|migration|schema|sql|model|backend|service|auth) ]]; then
        echo "backend"
        return
    fi

    # Default
    echo "general"
}
