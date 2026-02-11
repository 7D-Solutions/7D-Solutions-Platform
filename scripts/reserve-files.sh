#!/usr/bin/env bash
# File Reservation Tool - MCP Agent Mail Integration
# Provides advisory file locking to prevent concurrent edit conflicts
# Usage: ./scripts/reserve-files.sh <action> <files...>

# Note: -e flag omitted for graceful error handling with jq pipelines
set -uo pipefail

# Source shared project configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AGENTCORE_ROOT="$(dirname "$SCRIPT_DIR")"
source "$SCRIPT_DIR/lib/project-config.sh"

# Mail server configuration
MAIL_SERVER="${MAIL_SERVER:-http://127.0.0.1:8765}"
MCP_AGENT_MAIL_DIR="${MCP_AGENT_MAIL_DIR:-$HOME/mcp_agent_mail}"
TOKEN_FILE="$MCP_AGENT_MAIL_DIR/.env"

# Configuration
PROJECT_KEY="${PROJECT_KEY:-$MAIL_PROJECT_KEY}"
AGENT_NAME="${AGENT_NAME:-}"
# Default TTL tuned for async multi-agent workflows (Phase 2)
DEFAULT_TTL="${DEFAULT_TTL:-1800}"  # seconds (default 30 minutes)
# Warning threshold for upcoming expirations (seconds)
TTL_WARN_THRESHOLD="${TTL_WARN_THRESHOLD:-900}"  # default 15 minutes
# Directory for tracking blocked requesters (for auto-notify on release)
PENDING_DIR="${PENDING_DIR:-$AGENTCORE_ROOT/.beads/reserve-pending}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Load token
if [ ! -f "$TOKEN_FILE" ]; then
    echo -e "${RED}Error: Token file not found at $TOKEN_FILE${NC}"
    echo "Is the MCP Agent Mail server running?"
    exit 1
fi
TOKEN=$(grep HTTP_BEARER_TOKEN "$TOKEN_FILE" | cut -d'=' -f2)

# Get current agent name
get_agent_name() {
    if [ -z "$AGENT_NAME" ]; then
        AGENT_NAME=$("$SCRIPT_DIR/agent-mail-helper.sh" whoami 2>/dev/null || echo "unknown")
    fi
    echo "$AGENT_NAME"
}

# [bd-389b] Record a blocked requester for later notification on release
# Args: $1=blocked_agent, $2=holder_agent, $3=path_pattern
record_blocked_requester() {
    local blocked_agent="$1"
    local holder_agent="$2"
    local path_pattern="$3"

    mkdir -p "$PENDING_DIR"

    # Create a pending file keyed by holder+path hash
    local key=$(echo "${holder_agent}:${path_pattern}" | md5sum | cut -c1-12)
    local pending_file="$PENDING_DIR/${key}.pending"

    # Append requester if not already recorded (avoid duplicates)
    if [ -f "$pending_file" ]; then
        if ! grep -q "^${blocked_agent}$" "$pending_file" 2>/dev/null; then
            echo "$blocked_agent" >> "$pending_file"
        fi
    else
        # New pending file: store metadata on first line, then requesters
        echo "# holder:$holder_agent path:$path_pattern" > "$pending_file"
        echo "$blocked_agent" >> "$pending_file"
    fi
}

# [bd-389b] Notify pending requesters when files are released
# Args: $1=releasing_agent, $2+=released_paths (optional, empty means all)
notify_pending_requesters() {
    local releasing_agent="$1"
    shift
    local released_paths=("$@")

    [ ! -d "$PENDING_DIR" ] && return 0

    local notified_count=0

    for pending_file in "$PENDING_DIR"/*.pending; do
        [ ! -f "$pending_file" ] && continue

        # Read metadata from first line
        local metadata=$(head -1 "$pending_file")
        local holder=$(echo "$metadata" | sed -n 's/.*holder:\([^ ]*\).*/\1/p')
        local path=$(echo "$metadata" | sed -n 's/.*path:\([^ ]*\).*/\1/p')

        # Only process if this agent is the holder
        [ "$holder" != "$releasing_agent" ] && continue

        # If specific paths given, check for match
        if [ ${#released_paths[@]} -gt 0 ]; then
            local path_match=false
            for rp in "${released_paths[@]}"; do
                if [[ "$rp" == "$path" ]] || [[ "$rp" == "$path"* ]] || [[ "$path" == "$rp"* ]]; then
                    path_match=true
                    break
                fi
            done
            [ "$path_match" = false ] && continue
        fi

        # Notify each blocked requester (skip comment lines)
        while IFS= read -r requester; do
            [[ "$requester" == "#"* ]] && continue
            [[ -z "$requester" ]] && continue

            MAIL_SENDER_NAME="SystemNotify" "$SCRIPT_DIR/agent-mail-helper.sh" send "$requester" "File reservation released" \
                "$releasing_agent released: $path. You previously requested this path. It may now be available." \
                >/dev/null 2>&1 || true
            ((notified_count++))
        done < "$pending_file"

        # Remove the pending file after notifying
        rm -f "$pending_file"
    done

    if [ $notified_count -gt 0 ]; then
        echo -e "${BLUE}Notified $notified_count blocked requester(s)${NC}"
    fi

    return 0
}

# Make MCP API call
mcp_call() {
    local method=$1
    local tool_name=$2
    local arguments=$3

    local payload=$(cat <<EOF
{
  "jsonrpc": "2.0",
  "method": "$method",
  "params": {
    "name": "$tool_name",
    "arguments": $arguments
  },
  "id": $(date +%s)
}
EOF
)

    curl -s -X POST "$MAIL_SERVER/mcp" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d "$payload"
}

# Make MCP resource call
mcp_resource() {
    local uri=$1

    local payload=$(cat <<EOF
{
  "jsonrpc": "2.0",
  "method": "resources/read",
  "params": {
    "uri": "$uri"
  },
  "id": $(date +%s)
}
EOF
)

    curl -s -X POST "$MAIL_SERVER/mcp" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d "$payload"
}

usage() {
    cat <<EOF
File Reservation Tool - Advisory file locking via MCP Agent Mail

USAGE:
    $0 reserve <file-patterns...>   Reserve files (exclusive, ${DEFAULT_TTL}s TTL)
    $0 request <file-pattern>       Request access to a held file (notifies holder)
    $0 check <file-patterns...>     Check if files are available
    $0 release [file-patterns...]   Release reservations (all if no patterns)
    $0 release --id <ID> [<ID>...]  Release specific reservation IDs
    $0 list                         List your active reservations
    $0 list-all                     List all active reservations (all agents)
    $0 renew [extend-seconds]       Renew all reservations (default: +${DEFAULT_TTL}s)

EXAMPLES:
    $0 reserve src/app.py           Reserve single file
    $0 reserve 'src/**'             Reserve directory tree
    $0 reserve 'frontend:src/*'     Reserve files in specific repo (multi-repo products)
    $0 reserve '*:config.json'      Reserve file across all repos (wildcard prefix)
    $0 request src/app.py           Request access (notifies holder, queues you)
    $0 request src/app.py --reason 'need to fix bug #123'
    $0 check 'docs/**'              Check if docs are available
    $0 release src/app.py           Release specific file
    $0 release                      Release all your reservations
    $0 release --id 23              Release reservation ID 23
    $0 release --id 23 24 25        Release multiple IDs
    $0 renew 7200                   Extend TTL by 2 hours

MULTI-REPO COORDINATION:
    When a product is configured (.agent-mail-project-id marker file exists),
    patterns can include a repo prefix for cross-repo coordination:
        - 'frontend:src/*' targets frontend repo only
        - 'backend:api/*' targets backend repo only
        - '*:shared/*' targets shared paths across all repos

    Product-level conflict detection prevents:
        - Same-repo conflicts (e.g., frontend:src/* vs frontend:src/api.ts)
        - Cross-repo wildcard conflicts (e.g., *:src/* blocks all src paths)

ENVIRONMENT:
    PROJECT_KEY             Project path (default: from project config)
    AGENT_NAME              Your agent name (default: from agent-mail-helper)
    BYPASS_RESERVATION      Set to 1 to bypass checks (advisory mode)
    AUTO_RELEASE_OWN_STALE  Set to 1 to auto-release your old reservations on self-conflict (default: 0)

NOTE: File reservations are ADVISORY - they prevent conflicts but are not enforced.
      Use BYPASS_RESERVATION=1 for optional enforcement in testing.

EXIT CODES:
    0 - Success
    1 - General error
    5 - Conflict with another agent
    6 - Self-conflict (duplicate reservation detected)

EOF
    exit 1
}

# Check if reservation bypass is enabled
check_bypass() {
    if [[ "${BYPASS_RESERVATION:-0}" == "1" ]]; then
        echo -e "${YELLOW}⚠️  Reservation bypass enabled (BYPASS_RESERVATION=1)${NC}"
        return 0  # Bypass active
    fi
    return 1  # No bypass
}

# Get product UID from marker file if it exists
get_product_uid() {
    local marker_file="$AGENTCORE_ROOT/.agent-mail-project-id"
    if [ -f "$marker_file" ]; then
        cat "$marker_file" | tr -d '\n'
    fi
}

# Check for product-level conflicts across all repos in the product
check_product_conflicts() {
    local agent=$1
    local product_uid=$(get_product_uid)

    if [ -z "$product_uid" ]; then
        # No product configured, skip product-level checks
        return 0
    fi

    shift
    local requested_paths=("$@")

    # Query product-scoped reservations
    local response=$(mcp_resource "resource://file_reservations/product/$product_uid?active_only=true")

    # Check if resource exists
    local error_msg=$(echo "$response" | jq -r '.error.message // empty' 2>/dev/null)
    if [ -n "$error_msg" ]; then
        # Resource doesn't exist yet (expected for single-repo setup)
        return 0
    fi

    local all_reservations=$(echo "$response" | jq -r '.result.contents[0].text' 2>/dev/null || echo "[]")

    if [ "$all_reservations" = "[]" ] || [ -z "$all_reservations" ] || [ "$all_reservations" = "null" ]; then
        return 0  # No reservations in product
    fi

    # Check for cross-repo conflicts
    local has_conflict=false
    for requested in "${requested_paths[@]}"; do
        # Parse repo prefix if present (e.g., "frontend:src/*" -> "frontend" and "src/*")
        local req_repo=""
        local req_pattern="$requested"
        if [[ "$requested" == *:* ]]; then
            req_repo="${requested%%:*}"
            req_pattern="${requested#*:}"
        fi

        while IFS= read -r reservation; do
            if [ -z "$reservation" ] || [ "$reservation" = "null" ]; then
                continue
            fi

            local res_agent=$(echo "$reservation" | jq -r '.agent')
            local res_project=$(echo "$reservation" | jq -r '.project_slug')
            local res_pattern=$(echo "$reservation" | jq -r '.path_pattern')
            local res_exclusive=$(echo "$reservation" | jq -r '.exclusive')

            # Skip own reservations
            if [ "$res_agent" = "$agent" ]; then
                continue
            fi

            # Parse reservation's repo prefix if present
            local res_repo=""
            local res_path="$res_pattern"
            if [[ "$res_pattern" == *:* ]]; then
                res_repo="${res_pattern%%:*}"
                res_path="${res_pattern#*:}"
            fi

            # Only check for conflict if:
            # 1. Both have no repo prefix (same-project conflict)
            # 2. Both have same repo prefix (same-repo conflict)
            # 3. At least one uses wildcard repo prefix (e.g., "*:src/*")
            local should_check=false
            if [[ -z "$req_repo" && -z "$res_repo" ]]; then
                # Both in default repo, check if same project
                local current_slug=$(echo "$PROJECT_KEY" | sed 's/^\/\+//' | tr '/' '-' | tr '[:upper:]' '[:lower:]')
                if [ "$res_project" = "$current_slug" ]; then
                    should_check=true
                fi
            elif [[ "$req_repo" = "$res_repo" ]]; then
                should_check=true
            elif [[ "$req_repo" = "*" || "$res_repo" = "*" ]]; then
                should_check=true
            fi

            if [ "$should_check" = true ]; then
                # Check pattern overlap (simple heuristic)
                local pattern_to_check="$req_pattern"
                local existing_pattern="$res_path"

                if [[ "$pattern_to_check" == "$existing_pattern" ]] || \
                   [[ "$pattern_to_check" == "$existing_pattern"* ]] || \
                   [[ "$existing_pattern" == "$pattern_to_check"* ]]; then
                    has_conflict=true
                    echo -e "${YELLOW}⚠️  Product-level conflict detected:${NC}"
                    echo -e "  Agent ${BLUE}$res_agent${NC} in project ${BLUE}$res_project${NC}"
                    echo -e "  holds ${BLUE}$res_pattern${NC} (exclusive: $res_exclusive)"
                    echo -e "  Your requested pattern: ${BLUE}$requested${NC}"
                    echo ""
                fi
            fi
        done < <(echo "$all_reservations" | jq -c '.[]')
    done

    if [ "$has_conflict" = true ]; then
        echo -e "${YELLOW}Cross-repo conflicts detected in product '$product_uid'.${NC}"
        echo "Consider coordinating with the other agents or using different patterns."
        echo ""
        return 5  # Conflict exit code
    fi

    return 0
}

# Check for self-conflicts (duplicate reservations)
# Returns exit code 6 if self-conflict detected, but allows operation to continue (advisory)
# With AUTO_RELEASE_OWN_STALE=1, automatically releases conflicting older reservations
check_self_conflicts() {
    local agent=$1
    shift
    local requested_paths=("$@")

    # Get current reservations for this agent
    local slug=$(echo "$PROJECT_KEY" | sed 's/^\/\+//' | tr '/' '-' | tr '[:upper:]' '[:lower:]')
    local response=$(mcp_resource "resource://file_reservations/$slug?active_only=true")

    # Parse full reservation data for current agent (including IDs)
    local all_reservations=$(echo "$response" | jq -r '.result.contents[0].text' 2>/dev/null || echo "[]")
    local agent_reservations=$(echo "$all_reservations" | jq -r --arg agent "$agent" '[.[] | select(.agent == $agent)]' 2>/dev/null || echo "[]")

    if [ "$agent_reservations" = "[]" ] || [ -z "$agent_reservations" ]; then
        return 0  # No existing reservations, no conflict
    fi

    # Check for overlaps and collect conflicting reservation IDs
    local has_overlap=false
    local conflicting_ids=()
    local conflicting_paths=()

    for requested in "${requested_paths[@]}"; do
        while IFS= read -r reservation; do
            if [ -z "$reservation" ] || [ "$reservation" = "null" ]; then
                continue
            fi

            local existing=$(echo "$reservation" | jq -r '.path_pattern')
            local res_id=$(echo "$reservation" | jq -r '.id')

            if [ -z "$existing" ] || [ "$existing" = "null" ]; then
                continue
            fi

            # Simple overlap check: exact match or glob pattern similarity
            if [[ "$requested" == "$existing" ]] || \
               [[ "$requested" == "$existing"* ]] || \
               [[ "$existing" == "$requested"* ]]; then
                has_overlap=true
                conflicting_ids+=("$res_id")
                conflicting_paths+=("$existing")

                echo -e "${YELLOW}⚠️  Self-conflict detected:${NC}"
                echo -e "  You already hold a reservation for: ${BLUE}$existing${NC} (ID: $res_id)"
                echo -e "  Requested pattern: ${BLUE}$requested${NC}"
                echo ""
            fi
        done < <(echo "$agent_reservations" | jq -c '.[]')
    done

    if [ "$has_overlap" = true ]; then
        # Check if auto-release is enabled
        if [[ "${AUTO_RELEASE_OWN_STALE:-0}" == "1" ]]; then
            echo -e "${GREEN}✓ AUTO_RELEASE_OWN_STALE enabled${NC}"
            echo "  Automatically releasing ${#conflicting_ids[@]} conflicting reservation(s)..."
            echo ""

            # Release conflicting reservations by ID
            if [ ${#conflicting_ids[@]} -gt 0 ]; then
                # Build release command arguments
                local release_args=("--id" "${conflicting_ids[@]}")

                # Log what's being released
                echo -e "${BLUE}Releasing:${NC}"
                for i in "${!conflicting_ids[@]}"; do
                    echo "  - ID ${conflicting_ids[$i]}: ${conflicting_paths[$i]}"
                done
                echo ""

                # Call release with ID arguments (suppress normal output, we're logging ourselves)
                release_files "${release_args[@]}" >/dev/null 2>&1

                echo -e "${GREEN}✓ Auto-released old reservations${NC}"
                echo "  Proceeding with new reservation..."
                echo ""
            fi

            return 0  # Auto-release successful, no error
        else
            echo -e "${YELLOW}This is a duplicate reservation. Consider:${NC}"
            echo "  1. Release existing reservation first (./scripts/reserve-files.sh list, then release)"
            echo "  2. Continue anyway (advisory system allows this)"
            echo "  3. Use different path pattern"
            echo "  4. Enable auto-release: AUTO_RELEASE_OWN_STALE=1"
            echo ""
            echo -e "${YELLOW}Proceeding with reservation (advisory mode)...${NC}"
            echo ""
            return 6  # Self-conflict exit code
        fi
    fi

    return 0
}

# Reserve files
reserve_files() {
    local paths=()
    local reason="File editing session"

    # Parse arguments
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --reason)
                shift
                reason="$1"
                shift
                ;;
            *)
                paths+=("$1")
                shift
                ;;
        esac
    done

    if [[ ${#paths[@]} -eq 0 ]]; then
        echo -e "${RED}Error: No file patterns specified${NC}"
        usage
    fi

    local agent=$(get_agent_name)

    # Check for product-level conflicts first (across repos)
    local product_conflict_code=0
    set +e
    check_product_conflicts "$agent" "${paths[@]}"
    product_conflict_code=$?
    set +e  # remain non-strict; we handle exit codes explicitly

    if [ $product_conflict_code -eq 5 ]; then
        # Product-level conflict detected
        exit 5
    fi

    # Check for self-conflicts (advisory warning)
    # Temporarily disable exit-on-error to capture the return code
    local self_conflict_code=0
    set +e
    check_self_conflicts "$agent" "${paths[@]}"
    self_conflict_code=$?
    set +e  # remain non-strict; we handle exit codes explicitly

    # Convert paths array to JSON array
    local paths_json=$(printf '%s\n' "${paths[@]}" | jq -R . | jq -s .)

    # Build arguments JSON
    local args=$(cat <<EOF
{
  "project_key": "$PROJECT_KEY",
  "agent_name": "$agent",
  "paths": $paths_json,
  "ttl_seconds": $DEFAULT_TTL,
  "exclusive": true,
  "reason": "$reason"
}
EOF
)

    echo "Reserving files for $agent..."
    echo "Patterns: ${paths[*]}"
    echo "TTL: ${DEFAULT_TTL}s"
    echo ""

    local response=$(mcp_call "tools/call" "file_reservation_paths" "$args")

    # Check for errors
    local error=$(echo "$response" | jq -r '.error.message // empty')
    if [ -n "$error" ]; then
        echo -e "${RED}Error: $error${NC}"
        exit 1
    fi

    # Parse response - use structuredContent if available, otherwise parse text field
    local result_data
    if echo "$response" | jq -e '.result.structuredContent' >/dev/null 2>&1; then
        result_data=$(echo "$response" | jq -r '.result.structuredContent')
    else
        result_data=$(echo "$response" | jq -r '.result.content[0].text')
    fi

    local granted=$(echo "$result_data" | jq -r '.granted // empty')
    local conflicts=$(echo "$result_data" | jq -r '.conflicts // empty')

    if [ -n "$granted" ] && [ "$granted" != "null" ] && [ "$granted" != "[]" ]; then
        echo -e "${GREEN}✓ Reserved:${NC}"
        echo "$granted" | jq -r '.[] | "  - \(.path_pattern) (ID: \(.id), expires: \(.expires_ts))"'
    fi

    if [ -n "$conflicts" ] && [ "$conflicts" != "null" ] && [ "$conflicts" != "[]" ]; then
        echo -e "${YELLOW}⚠️  Conflicts detected:${NC}"
        echo "$conflicts" | jq -r '.[] | "  - \(.path): held by \(.holders | map(.agent) | join(", "))"'
        echo ""
        echo -e "${YELLOW}Another agent is working on these files. Consider:${NC}"
        echo "  1. Wait for them to release"
        echo "  2. Coordinate via agent mail"
        echo "  3. Work on different files"

        # Notify current holders to speed up coordination
        # Collect unique holders to avoid duplicate notifications
        local holders=()
        local holders_set=""
        while IFS= read -r h; do
            [[ -z "$h" || "$h" == "null" ]] && continue
            if [[ " $holders_set " != *" $h "* ]]; then
                holders+=("$h")
                holders_set="$holders_set $h"
            fi
        done < <(echo "$conflicts" | jq -r '.[] | .holders[].agent')

        for h in "${holders[@]}"; do
            if [[ -n "$h" && "$h" != "null" && "$h" != "$agent" ]]; then
                MAIL_SENDER_NAME="SystemNotify" "$SCRIPT_DIR/agent-mail-helper.sh" send "$h" "Reservation conflict notice" \
                    "$agent attempted to reserve: ${paths[*]}. You currently hold one or more matching paths. If free, please release or reply with timing. Project: $PROJECT_KEY" >/dev/null 2>&1 || true
            fi
        done

        # [bd-389b] Record blocked requester for auto-notify on release
        while IFS= read -r conflict_info; do
            [[ -z "$conflict_info" ]] && continue
            local conflict_path=$(echo "$conflict_info" | jq -r '.path')
            while IFS= read -r holder_info; do
                [[ -z "$holder_info" ]] && continue
                local holder_agent=$(echo "$holder_info" | jq -r '.agent')
                [[ "$holder_agent" == "$agent" ]] && continue
                record_blocked_requester "$agent" "$holder_agent" "$conflict_path"
            done < <(echo "$conflict_info" | jq -c '.holders[]')
        done < <(echo "$conflicts" | jq -c '.[]')

        exit 5
    fi

    # Return self-conflict code if detected (advisory warning)
    if [ $self_conflict_code -eq 6 ]; then
        exit 6
    fi
}

# Check if files are available
check_files() {
    local paths=("$@")
    if [[ ${#paths[@]} -eq 0 ]]; then
        echo -e "${RED}Error: No file patterns specified${NC}"
        usage
    fi

    if check_bypass; then
        echo -e "${GREEN}✓ Check bypassed - proceeding${NC}"
        return 0
    fi

    local agent=$(get_agent_name)

    # [bd-2b8] Check for product-level conflicts first (cross-repo)
    local product_conflict_code=0
    set +e
    check_product_conflicts "$agent" "${paths[@]}"
    product_conflict_code=$?
    set -e

    if [ $product_conflict_code -eq 5 ]; then
        # Product-level conflict detected (already reported by check_product_conflicts)
        return 1
    fi

    # Convert paths array to JSON array
    local paths_json=$(printf '%s\n' "${paths[@]}" | jq -R . | jq -s .)

    # Build arguments JSON - use shared mode to check without blocking others
    local args=$(cat <<EOF
{
  "project_key": "$PROJECT_KEY",
  "agent_name": "$agent",
  "paths": $paths_json,
  "ttl_seconds": 60,
  "exclusive": false,
  "reason": "Availability check"
}
EOF
)

    echo "Checking availability: ${paths[*]}"

    local response=$(mcp_call "tools/call" "file_reservation_paths" "$args")

    # Parse response
    local result_data
    if echo "$response" | jq -e '.result.structuredContent' >/dev/null 2>&1; then
        result_data=$(echo "$response" | jq -r '.result.structuredContent')
    else
        result_data=$(echo "$response" | jq -r '.result.content[0].text')
    fi

    local conflicts=$(echo "$result_data" | jq -r '.conflicts // empty')

    if [ -n "$conflicts" ] && [ "$conflicts" != "null" ] && [ "$conflicts" != "[]" ]; then
        echo -e "${YELLOW}⚠️  Files are currently reserved:${NC}"
        echo "$conflicts" | jq -r '.[] | "  - \(.path): held by \(.holders | map(.agent) | join(", "))"'

        # Release our temporary check reservation
        release_files "${paths[@]}" >/dev/null 2>&1 || true

        return 1
    else
        echo -e "${GREEN}✓ Files are available${NC}"

        # Release our temporary check reservation
        release_files "${paths[@]}" >/dev/null 2>&1 || true

        return 0
    fi
}

# [bd-2ba9] Request access to a held file - notifies holder, queues requester
# Usage: request_files <path> [--reason 'why needed']
request_files() {
    local path=""
    local reason=""

    # Parse arguments
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --reason)
                shift
                reason="$1"
                shift
                ;;
            *)
                if [ -z "$path" ]; then
                    path="$1"
                fi
                shift
                ;;
        esac
    done

    if [ -z "$path" ]; then
        echo -e "${RED}Error: No file pattern specified${NC}"
        echo "Usage: $0 request <path> [--reason 'why needed']"
        exit 1
    fi

    local agent=$(get_agent_name)

    # Check who holds this path
    local slug=$(echo "$PROJECT_KEY" | sed 's/^\/\+//' | tr '/' '-' | tr '[:upper:]' '[:lower:]')
    local response=$(mcp_resource "resource://file_reservations/$slug?active_only=true")
    local all_reservations=$(echo "$response" | jq -r '.result.contents[0].text' 2>/dev/null || echo "[]")

    if [ "$all_reservations" = "[]" ] || [ -z "$all_reservations" ] || [ "$all_reservations" = "null" ]; then
        echo -e "${GREEN}✓ Path '$path' is available - use 'reserve' instead${NC}"
        return 0
    fi

    # Find holders for this path
    local holders=()
    local holder_found=false

    while IFS= read -r reservation; do
        [ -z "$reservation" ] || [ "$reservation" = "null" ] && continue

        local res_agent=$(echo "$reservation" | jq -r '.agent')
        local res_pattern=$(echo "$reservation" | jq -r '.path_pattern')

        # Skip own reservations
        [ "$res_agent" = "$agent" ] && continue

        # Check for pattern match
        if [[ "$path" == "$res_pattern" ]] || \
           [[ "$path" == "$res_pattern"* ]] || \
           [[ "$res_pattern" == "$path"* ]]; then
            holder_found=true
            # Avoid duplicates
            if [[ ${#holders[@]} -eq 0 ]] || [[ ! " ${holders[*]} " =~ " ${res_agent} " ]]; then
                holders+=("$res_agent")
            fi
        fi
    done < <(echo "$all_reservations" | jq -c '.[]')

    if [ "$holder_found" = false ]; then
        echo -e "${GREEN}✓ Path '$path' is available - use 'reserve' instead${NC}"
        return 0
    fi

    # Path is held - record request and notify holders
    echo -e "${YELLOW}Path '$path' is held by: ${holders[*]}${NC}"
    echo "Sending request notification..."

    local reason_text=""
    if [ -n "$reason" ]; then
        reason_text=" Reason: $reason"
    fi

    for holder in "${holders[@]}"; do
        # Record this agent as waiting for holder to release
        record_blocked_requester "$agent" "$holder" "$path"

        # Notify holder via SystemNotify
        MAIL_SENDER_NAME="SystemNotify" "$SCRIPT_DIR/agent-mail-helper.sh" send "$holder" "File access requested" \
            "$agent is waiting for: $path.$reason_text Please release when done, or reply with timing." \
            >/dev/null 2>&1 || true
    done

    echo -e "${GREEN}✓ Request sent to ${#holders[@]} holder(s)${NC}"
    echo "  You'll be notified when the path is released."

    return 0
}

# Release reservations
release_files() {
    local args_array=("$@")
    local agent=$(get_agent_name)
    local use_ids=false
    local ids=()
    local paths=()

    # Parse arguments: check for --id flag
    local i=0
    while [ $i -lt ${#args_array[@]} ]; do
        if [[ "${args_array[$i]}" == "--id" ]]; then
            use_ids=true
            # Collect all following numeric arguments as IDs
            ((i++))
            while [ $i -lt ${#args_array[@]} ] && [[ "${args_array[$i]}" =~ ^[0-9]+$ ]]; do
                ids+=("${args_array[$i]}")
                ((i++))
            done
        else
            paths+=("${args_array[$i]}")
            ((i++))
        fi
    done

    # Build arguments JSON based on mode
    local args
    if [ "$use_ids" = true ]; then
        if [[ ${#ids[@]} -eq 0 ]]; then
            echo -e "${RED}Error: --id flag requires at least one ID${NC}"
            exit 1
        fi
        echo "Releasing reservation IDs: ${ids[*]}"
        local ids_json=$(printf '%s\n' "${ids[@]}" | jq -R 'tonumber' | jq -s .)
        args=$(cat <<EOF
{
  "project_key": "$PROJECT_KEY",
  "agent_name": "$agent",
  "file_reservation_ids": $ids_json
}
EOF
)
    elif [[ ${#paths[@]} -eq 0 ]]; then
        echo "Releasing ALL reservations for $agent..."
        args=$(cat <<EOF
{
  "project_key": "$PROJECT_KEY",
  "agent_name": "$agent"
}
EOF
)
    else
        echo "Releasing: ${paths[*]}"
        local paths_json=$(printf '%s\n' "${paths[@]}" | jq -R . | jq -s .)
        args=$(cat <<EOF
{
  "project_key": "$PROJECT_KEY",
  "agent_name": "$agent",
  "paths": $paths_json
}
EOF
)
    fi

    local response=$(mcp_call "tools/call" "release_file_reservations" "$args")

    # Check for errors
    local error=$(echo "$response" | jq -r '.error.message // empty')
    if [ -n "$error" ]; then
        echo -e "${RED}Error: $error${NC}"
        exit 1
    fi

    # Parse response
    local result_data
    if echo "$response" | jq -e '.result.structuredContent' >/dev/null 2>&1; then
        result_data=$(echo "$response" | jq -r '.result.structuredContent')
    else
        result_data=$(echo "$response" | jq -r '.result.content[0].text')
    fi

    local released=$(echo "$result_data" | jq -r '.released // 0')
    echo -e "${GREEN}✓ Released $released reservation(s)${NC}"

    # [bd-389b] Notify any blocked requesters that files are now available
    if [ ${#paths[@]} -gt 0 ]; then
        notify_pending_requesters "$agent" "${paths[@]}"
    else
        notify_pending_requesters "$agent"
    fi
}

# List active reservations for current agent
list_reservations() {
    local agent=$(get_agent_name)

    # Convert project key to slug for resource URI
    local slug=$(echo "$PROJECT_KEY" | sed 's/^\/\+//' | tr '/' '-' | tr '[:upper:]' '[:lower:]')

    echo "Active reservations for $agent:"
    echo "================================"

    local response=$(mcp_resource "resource://file_reservations/$slug?active_only=true")

    # Check for errors
    local error=$(echo "$response" | jq -r '.error.message // empty')
    if [ -n "$error" ]; then
        echo -e "${YELLOW}(No reservations or error: $error)${NC}"
        return 0
    fi

    # Parse and filter for current agent
    local reservations=$(echo "$response" | jq -r '.result.contents[0].text' | \
        jq -r --arg agent "$agent" '[.[] | select(.agent == $agent)]')

    if [ -z "$reservations" ] || [ "$reservations" = "null" ] || [ "$reservations" = "[]" ]; then
        echo "(No active reservations)"
    else
        # Show reservations with waiting count
        while IFS= read -r res_line; do
            [ -z "$res_line" ] && continue
            local res_id=$(echo "$res_line" | jq -r '.id')
            local res_path=$(echo "$res_line" | jq -r '.path_pattern')
            local res_exclusive=$(echo "$res_line" | jq -r '.exclusive')
            local res_expires=$(echo "$res_line" | jq -r '.expires_ts')

            # Count waiting requesters for this path
            local waiting_count=0
            if [ -d "$PENDING_DIR" ]; then
                shopt -s nullglob
                for pf in "$PENDING_DIR"/*.pending; do
                    [ ! -f "$pf" ] && continue
                    local pf_meta=$(head -1 "$pf")
                    local pf_holder=$(echo "$pf_meta" | sed -n 's/.*holder:\([^ ]*\).*/\1/p')
                    local pf_path=$(echo "$pf_meta" | sed -n 's/.*path:\([^ ]*\).*/\1/p')
                    if [ "$pf_holder" = "$agent" ] && [ "$pf_path" = "$res_path" ]; then
                        # Count non-comment lines (requesters)
                        waiting_count=$(grep -v '^#' "$pf" | grep -c . || echo 0)
                    fi
                done
                shopt -u nullglob
            fi

            # Format output
            local waiting_info=""
            if [ "$waiting_count" -gt 0 ]; then
                waiting_info=" ${YELLOW}⏳ $waiting_count waiting${NC}"
            fi
            echo -e "  [$res_id] $res_path (exclusive: $res_exclusive, expires: $res_expires)$waiting_info"
        done < <(echo "$reservations" | jq -c '.[]')
    fi
}

# List all active reservations (all agents)
list_all_reservations() {
    # Convert project key to slug for resource URI
    local slug=$(echo "$PROJECT_KEY" | sed 's/^\/\+//' | tr '/' '-' | tr '[:upper:]' '[:lower:]')

    echo "All active reservations in project:"
    echo "===================================="

    local response=$(mcp_resource "resource://file_reservations/$slug?active_only=true")

    # Check for errors
    local error=$(echo "$response" | jq -r '.error.message // empty')
    if [ -n "$error" ]; then
        echo -e "${YELLOW}(No reservations or error: $error)${NC}"
        return 0
    fi

    # Parse all reservations
    local reservations=$(echo "$response" | jq -r '.result.contents[0].text')

    if [ -z "$reservations" ] || [ "$reservations" = "null" ] || [ "$reservations" = "[]" ]; then
        echo "(No active reservations)"
    else
        echo "$reservations" | jq -r '.[] | "  [\(.agent)] \(.path_pattern) (ID: \(.id), exclusive: \(.exclusive), expires: \(.expires_ts))"'
    fi
}

# Renew reservations
renew_reservations() {
    local extend_seconds="${1:-$DEFAULT_TTL}"
    local agent=$(get_agent_name)

    echo "Renewing reservations for $agent, extending by ${extend_seconds}s..."

    # Build arguments JSON (omit file_reservation_ids to renew all)
    local args=$(cat <<EOF
{
  "project_key": "$PROJECT_KEY",
  "agent_name": "$agent",
  "extend_seconds": $extend_seconds
}
EOF
)

    local response=$(mcp_call "tools/call" "renew_file_reservations" "$args")

    # Check for errors
    local error=$(echo "$response" | jq -r '.error.message // empty')
    if [ -n "$error" ]; then
        echo -e "${RED}Error: $error${NC}"
        exit 1
    fi

    # Parse response
    local result_data
    if echo "$response" | jq -e '.result.structuredContent' >/dev/null 2>&1; then
        result_data=$(echo "$response" | jq -r '.result.structuredContent')
    else
        result_data=$(echo "$response" | jq -r '.result.content[0].text')
    fi

    local renewed=$(echo "$result_data" | jq -r '.renewed // 0')
    echo -e "${GREEN}✓ Renewed $renewed reservation(s)${NC}"
}

# Warn on reservations nearing expiry for current agent
warn_expiring() {
    local agent=$(get_agent_name)
    local slug
    slug=$(echo "$PROJECT_KEY" | sed 's|^/\\+||' | tr '/' '-' | tr '[:upper:]' '[:lower:]')

    local response=$(mcp_resource "resource://file_reservations/$slug?active_only=true")
    local reservations=$(echo "$response" | jq -r '.result.contents[0].text' 2>/dev/null)

    if [ -z "$reservations" ] || [ "$reservations" = "null" ] || [ "$reservations" = "[]" ]; then
        echo "No active reservations."
        return 0
    fi

    local now
    now=$(date -u +%s)
    local warned=0

    echo "Reservations expiring within $TTL_WARN_THRESHOLD seconds for $agent:"
    echo "------------------------------------------------------------------"

    echo "$reservations" | jq -c '.[]' | while read -r item; do
        local holder expires path id
        holder=$(echo "$item" | jq -r '.agent')
        expires=$(echo "$item" | jq -r '.expires_ts')
        path=$(echo "$item" | jq -r '.path_pattern')
        id=$(echo "$item" | jq -r '.id')

        [[ "$holder" != "$agent" ]] && continue
        local exp_epoch
        exp_epoch=$(date -d "$expires" +%s 2>/dev/null || date -j -f "%Y-%m-%dT%H:%M:%S%z" "$expires" +%s 2>/dev/null || echo "")
        [[ -z "$exp_epoch" ]] && continue

        local delta=$((exp_epoch - now))
        if [ $delta -le $TTL_WARN_THRESHOLD ] && [ $delta -gt 0 ]; then
            warned=1
            echo "  [ID $id] $path expires in ${delta}s at $expires"
        fi
    done

    if [ $warned -eq 0 ]; then
        echo "(None within threshold)"
    fi
}

# Main
main() {
    if [[ $# -eq 0 ]]; then
        usage
    fi

    local action="$1"
    shift

    case "$action" in
        reserve)
            reserve_files "$@"
            ;;
        request)
            request_files "$@"
            ;;
        check)
            check_files "$@"
            ;;
        release)
            release_files "$@"
            ;;
        list)
            list_reservations
            ;;
        list-all)
            list_all_reservations
            ;;
        renew)
            renew_reservations "$@"
            ;;
        warn-expiring)
            warn_expiring
            ;;
        help|--help|-h)
            usage
            ;;
        *)
            echo -e "${RED}Unknown action: $action${NC}"
            usage
            ;;
    esac
}

main "$@"
