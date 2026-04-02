#!/usr/bin/env bash
# generate-event-catalog.sh — Generate docs/event-catalog.md from source code.
#
# Extracts EVENT_TYPE constants, enum event_type patterns, inline enqueue
# event types, and consumer subscriptions to produce a comprehensive catalog.

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODULES_DIR="$PROJECT_ROOT/modules"
OUTPUT="$PROJECT_ROOT/docs/event-catalog.md"

module_from_path() {
    echo "$1" | sed -n "s|$MODULES_DIR/\([^/]*\)/.*|\1|p"
}

# ── Collect all events ───────────────────────────────────────────

declare -A EVENTS  # event_type → module|source_file

# 1. EVENT_TYPE_* constants
while IFS= read -r line; do
    file=$(echo "$line" | cut -d: -f1)
    event_type=$(echo "$line" | sed -n 's/.*= "\([^"]*\)".*/\1/p')
    if [ -n "$event_type" ]; then
        module=$(module_from_path "$file")
        short_file="${file#$PROJECT_ROOT/}"
        EVENTS["$event_type"]="$module|$short_file"
    fi
done < <(grep -rn 'pub const EVENT_TYPE' "$MODULES_DIR" --include="*.rs" || true)

# 2. Enum event_type patterns (production, maintenance, etc.)
while IFS= read -r line; do
    file=$(echo "$line" | cut -d: -f1)
    event_type=$(echo "$line" | sed -n 's/.*=> "\([^"]*\)".*/\1/p')
    if [ -n "$event_type" ] && [[ "$event_type" == *.* ]]; then
        module=$(module_from_path "$file")
        short_file="${file#$PROJECT_ROOT/}"
        [ -z "${EVENTS[$event_type]+x}" ] && EVENTS["$event_type"]="$module|$short_file"
    fi
done < <(grep -rn '=> "' "$MODULES_DIR" --include="*.rs" | grep -v '/tests/' | grep -v '/target' || true)

# 3. Inline event types from enqueue calls
while IFS= read -r line; do
    file=$(echo "$line" | cut -d: -f1)
    event_type=$(echo "$line" | sed -n 's/.*enqueue_event[^(]*([^,]*, *"\([^"]*\)".*/\1/p')
    if [ -n "$event_type" ]; then
        module=$(module_from_path "$file")
        short_file="${file#$PROJECT_ROOT/}"
        [ -z "${EVENTS[$event_type]+x}" ] && EVENTS["$event_type"]="$module|$short_file"
    fi
done < <(grep -rn 'enqueue_event.*"[a-z]' "$MODULES_DIR" --include="*.rs" | grep -v '/tests/' || true)

# 4. Multi-line enqueue event types
while IFS= read -r line; do
    # Extract file from grep -A context (format: file-linenum-content or file:linenum:content)
    file=$(echo "$line" | sed 's/[-:][0-9]*[-:].*$//' )
    event_type=$(echo "$line" | sed -n 's/.*"\([a-z][a-z_]*\.[a-z][a-z_.]*\)".*/\1/p')
    if [ -n "$event_type" ]; then
        module=$(module_from_path "$file")
        short_file="${file#$PROJECT_ROOT/}"
        [ -z "${EVENTS[$event_type]+x}" ] && EVENTS["$event_type"]="$module|$short_file"
    fi
done < <(grep -rn 'enqueue_event' "$MODULES_DIR" --include="*.rs" -A2 \
    | grep '"[a-z][a-z_]*\.[a-z]' | grep -v '/tests/' || true)

# ── Collect consumers ────────────────────────────────────────────

declare -A CONSUMERS  # subject → consuming_module

# SDK .consumer() calls
while IFS= read -r line; do
    file=$(echo "$line" | cut -d: -f1)
    subject=$(echo "$line" | sed -n 's/.*\.consumer("\([^"]*\)".*/\1/p')
    if [ -n "$subject" ]; then
        module=$(module_from_path "$file")
        existing="${CONSUMERS[$subject]:-}"
        if [ -n "$existing" ]; then
            CONSUMERS["$subject"]="$existing, $module"
        else
            CONSUMERS["$subject"]="$module"
        fi
    fi
done < <(grep -rn '\.consumer("' "$MODULES_DIR" --include="*.rs" | grep -v '/tests/' || true)

# let subject = "literal" in consumer files
while IFS= read -r line; do
    file=$(echo "$line" | cut -d: -f1)
    subject=$(echo "$line" | sed -n 's/.*let subject = "\([^"]*\)".*/\1/p')
    if [ -n "$subject" ] && [[ "$subject" == *.* ]] && [[ "$subject" != test.* ]]; then
        module=$(module_from_path "$file")
        existing="${CONSUMERS[$subject]:-}"
        if [ -n "$existing" ] && [[ "$existing" != *"$module"* ]]; then
            CONSUMERS["$subject"]="$existing, $module"
        elif [ -z "$existing" ]; then
            CONSUMERS["$subject"]="$module"
        fi
    fi
done < <(grep -rn 'let subject = "' "$MODULES_DIR" --include="*.rs" \
    | grep -v '/tests/' | grep -v '/outbox/' | grep -v 'publisher' | grep -v 'format!' || true)

# ── Generate markdown ────────────────────────────────────────────

cat > "$OUTPUT" << 'HEADER'
# Event Catalog

> **Generated from source code.** Do not edit manually — regenerate with:
> ```bash
> ./scripts/generate-event-catalog.sh
> ```

This catalog lists every event published across the platform, organized by
source module. Each entry shows the event type, the NATS subject it publishes
to, known consumers, and the source file containing the payload definition.

HEADER

# Group events by module
declare -A MODULE_EVENTS  # module → newline-separated event_types
for event_type in "${!EVENTS[@]}"; do
    info="${EVENTS[$event_type]}"
    module="${info%%|*}"
    existing="${MODULE_EVENTS[$module]:-}"
    if [ -n "$existing" ]; then
        MODULE_EVENTS["$module"]="$existing
$event_type"
    else
        MODULE_EVENTS["$module"]="$event_type"
    fi
done

# Publisher subject resolution (simplified)
resolve_nats_subject() {
    local module="$1"
    local event_type="$2"
    case "$module" in
        ar)
            if [[ "$event_type" == gl.* ]]; then
                local stripped="${event_type#gl.}"
                echo "gl.events.$stripped"
            else
                echo "ar.events.$event_type"
            fi
            ;;
        ap) echo "ap.events.$event_type" ;;
        payments) echo "payments.events.$event_type" ;;
        treasury) echo "treasury.events.$event_type" ;;
        subscriptions) echo "subscriptions.events.$event_type" ;;
        inventory) echo "inventory.events.$event_type" ;;
        gl) echo "gl.events.$event_type" ;;
        party) echo "party.events.$event_type" ;;
        shipping-receiving) echo "$event_type" ;;
        maintenance) echo "$event_type" ;;
        production) echo "$event_type" ;;
        workflow) echo "workflow.events.$event_type" ;;
        numbering) echo "numbering.events.$event_type" ;;
        notifications) echo "notifications.events.$event_type" ;;
        integrations) echo "integrations.events.$event_type" ;;
        workforce-competence) echo "workforce_competence.events.$event_type" ;;
        *) echo "$module.events.$event_type" ;;
    esac
}

# Sort modules alphabetically and generate sections
for module in $(echo "${!MODULE_EVENTS[@]}" | tr ' ' '\n' | sort); do
    echo "## $module" >> "$OUTPUT"
    echo "" >> "$OUTPUT"
    echo "| Event Type | NATS Subject | Consumers | Source |" >> "$OUTPUT"
    echo "|-----------|-------------|-----------|--------|" >> "$OUTPUT"

    # Sort event types within module
    while IFS= read -r event_type; do
        [ -z "$event_type" ] && continue
        info="${EVENTS[$event_type]}"
        source_file="${info#*|}"
        nats_subject=$(resolve_nats_subject "$module" "$event_type")

        # Find consumers for this subject (or direct event_type match)
        consumers="${CONSUMERS[$nats_subject]:-}"
        if [ -z "$consumers" ]; then
            consumers="${CONSUMERS[$event_type]:-}"
        fi
        [ -z "$consumers" ] && consumers="—"

        echo "| \`$event_type\` | \`$nats_subject\` | $consumers | \`$source_file\` |" >> "$OUTPUT"
    done < <(echo "${MODULE_EVENTS[$module]}" | sort)

    echo "" >> "$OUTPUT"
done

# Summary
total="${#EVENTS[@]}"
modules="${#MODULE_EVENTS[@]}"
consumers_count="${#CONSUMERS[@]}"

cat >> "$OUTPUT" << EOF
---

**Summary:** $total events across $modules modules, $consumers_count consumer subscriptions.

*Generated on $(date -u +%Y-%m-%dT%H:%M:%SZ)*
EOF

echo "Generated $OUTPUT with $total events across $modules modules"
