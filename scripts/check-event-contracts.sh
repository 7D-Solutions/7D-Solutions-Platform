#!/usr/bin/env bash
# check-event-contracts.sh — Verify every consumer subject has a matching publisher.
#
# Parses .consumer() SDK calls, bus.subscribe() patterns, and SUBJECT_*
# constants to find consumed subjects, then checks for matching
# EVENT_TYPE constants and outbox inserts in publisher code.
#
# Designed for CI: exits 0 on no mismatches, 1 on any mismatch.

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODULES_DIR="$PROJECT_ROOT/modules"
ERRORS=0

if [ ! -d "$MODULES_DIR" ]; then
    echo "ERROR: modules/ directory not found at $MODULES_DIR"
    exit 1
fi

echo "=== Event Contract Check ==="
echo ""

# ── Helper: extract module name from file path ───────────────────

module_from_path() {
    echo "$1" | sed -n "s|$MODULES_DIR/\([^/]*\)/.*|\1|p"
}

# ── Helper: map (module, event_type) → NATS subject ─────────────
#
# Each module's outbox publisher constructs subjects differently.
# This function returns all plausible subjects for the pair.

resolve_subjects() {
    local module="$1"
    local event_type="$2"

    # Direct subject (always possible)
    echo "$event_type"

    case "$module" in
        ar)
            echo "ar.events.$event_type"
            # AR also publishes gl.* events with special prefix stripping
            if [[ "$event_type" == gl.* ]]; then
                local stripped="${event_type#gl.}"
                echo "gl.events.$stripped"
            fi
            ;;
        ap)
            echo "ap.events.$event_type"
            ;;
        payments)
            echo "payments.events.$event_type"
            ;;
        treasury)
            echo "treasury.events.$event_type"
            ;;
        subscriptions)
            echo "subscriptions.events.$event_type"
            ;;
        inventory)
            echo "inventory.events.$event_type"
            # Inventory publisher may use event_type with module prefix
            echo "inventory.events.inventory.$event_type"
            ;;
        gl)
            echo "gl.events.$event_type"
            ;;
        shipping-receiving)
            # Uses event_type directly as subject
            ;;
        maintenance)
            # Uses event_type directly as subject
            ;;
        production)
            # Uses event_type directly as subject
            ;;
        fixed-assets)
            # Uses {aggregate_type}.{event_type}
            echo "fixed-assets.events.$event_type"
            ;;
        party)
            echo "party.events.$event_type"
            ;;
        workflow)
            echo "workflow.events.$event_type"
            ;;
        numbering)
            echo "numbering.events.$event_type"
            ;;
        notifications)
            echo "notifications.events.$event_type"
            ;;
        reporting)
            echo "reporting.events.$event_type"
            ;;
        *)
            echo "$module.events.$event_type"
            ;;
    esac
}

# ── Step 1: Collect consumed subjects ────────────────────────────

declare -A SDK_CONSUMERS       # subject → file  (SDK .consumer() — strict)
declare -A EXTENDED_CONSUMERS  # subject → file  (subscribe patterns — advisory)

# 1a. SDK .consumer("subject", ...) calls — STRICT
while IFS= read -r line; do
    file=$(echo "$line" | cut -d: -f1)
    subject=$(echo "$line" | sed -n 's/.*\.consumer("\([^"]*\)".*/\1/p')
    if [ -n "$subject" ]; then
        SDK_CONSUMERS["$subject"]="$file"
    fi
done < <(grep -rn '\.consumer("' "$MODULES_DIR" --include="*.rs" \
    | grep -v '/tests/' | grep -v '/test_' | grep -v '#\[doc' || true)

# 1b. let subject = "literal" in consumer source files — ADVISORY
while IFS= read -r line; do
    file=$(echo "$line" | cut -d: -f1)
    subject=$(echo "$line" | sed -n 's/.*let subject = "\([^"]*\)".*/\1/p')
    if [ -n "$subject" ] && [[ "$subject" == *.* ]] && [[ "$subject" != test.* ]]; then
        [ -z "${SDK_CONSUMERS[$subject]+x}" ] && EXTENDED_CONSUMERS["$subject"]="$file"
    fi
done < <(grep -rn 'let subject = "' "$MODULES_DIR" --include="*.rs" \
    | grep -v '/tests/' | grep -v '/outbox/' | grep -v 'publisher' \
    | grep -v 'format!' || true)

# 1c. const SUBJECT_* = "literal" patterns — ADVISORY
while IFS= read -r line; do
    file=$(echo "$line" | cut -d: -f1)
    subject=$(echo "$line" | sed -n 's/.*= "\([^"]*\)".*/\1/p')
    if [ -n "$subject" ] && [[ "$subject" != *">"* ]] && [[ "$subject" != *"*"* ]]; then
        [ -z "${SDK_CONSUMERS[$subject]+x}" ] && EXTENDED_CONSUMERS["$subject"]="$file"
    fi
done < <(grep -rn 'pub const SUBJECT' "$MODULES_DIR" --include="*.rs" | grep -v '/tests/' || true)

# 1d. Subject strings in consumer_task files — ADVISORY
while IFS= read -r line; do
    file=$(echo "$line" | cut -d: -f1)
    subject=$(echo "$line" | sed -n 's/.*let subject = "\([^"]*\)".*/\1/p')
    if [ -n "$subject" ] && [[ "$subject" == *.* ]] && [[ "$subject" != test.* ]]; then
        [ -z "${SDK_CONSUMERS[$subject]+x}" ] && EXTENDED_CONSUMERS["$subject"]="$file"
    fi
done < <(grep -rn 'let subject = "' "$MODULES_DIR" --include="*consumer*" | grep -v '/tests/' || true)

if [ ${#SDK_CONSUMERS[@]} -eq 0 ] && [ ${#EXTENDED_CONSUMERS[@]} -eq 0 ]; then
    echo "No consumer subjects found."
    exit 0
fi

echo "SDK .consumer() subjects: ${#SDK_CONSUMERS[@]}"
echo "Extended subscribe subjects: ${#EXTENDED_CONSUMERS[@]}"

# ── Step 2: Build set of all published NATS subjects ─────────────

declare -A PUBLISHED_SUBJECTS  # subject → 1

# 2a. EVENT_TYPE_* constants → resolve via owning module's publisher
while IFS= read -r line; do
    file=$(echo "$line" | cut -d: -f1)
    event_type=$(echo "$line" | sed -n 's/.*= "\([^"]*\)".*/\1/p')
    if [ -n "$event_type" ]; then
        module=$(module_from_path "$file")
        while IFS= read -r subj; do
            [ -n "$subj" ] && PUBLISHED_SUBJECTS["$subj"]=1
        done < <(resolve_subjects "$module" "$event_type")
    fi
done < <(grep -rn 'pub const EVENT_TYPE' "$MODULES_DIR" --include="*.rs" || true)

# 2b. Event type enums with string representations (e.g. production events)
while IFS= read -r line; do
    file=$(echo "$line" | cut -d: -f1)
    event_type=$(echo "$line" | sed -n 's/.*=> "\([^"]*\)".*/\1/p')
    if [ -n "$event_type" ] && [[ "$event_type" == *.* ]]; then
        module=$(module_from_path "$file")
        while IFS= read -r subj; do
            [ -n "$subj" ] && PUBLISHED_SUBJECTS["$subj"]=1
        done < <(resolve_subjects "$module" "$event_type")
    fi
done < <(grep -rn 'Self::.*=> "' "$MODULES_DIR" --include="*.rs" | grep -v '/tests/' || true)

# 2c. Inline event types from enqueue_event / enqueue_event_tx calls (same line)
while IFS= read -r line; do
    file=$(echo "$line" | cut -d: -f1)
    event_type=$(echo "$line" | sed -n 's/.*enqueue_event[^(]*([^,]*, *"\([^"]*\)".*/\1/p')
    if [ -n "$event_type" ]; then
        module=$(module_from_path "$file")
        while IFS= read -r subj; do
            [ -n "$subj" ] && PUBLISHED_SUBJECTS["$subj"]=1
        done < <(resolve_subjects "$module" "$event_type")
    fi
done < <(grep -rn 'enqueue_event.*"[a-z]' "$MODULES_DIR" --include="*.rs" | grep -v '/tests/' || true)

# 2c2. Multi-line: event_type strings near enqueue calls (string on separate line)
#      Matches lines like:  "payment.collection.requested",
#      in event/domain/outbox source files
while IFS= read -r line; do
    file=$(echo "$line" | cut -d: -f1)
    event_type=$(echo "$line" | sed -n 's/.*"\([a-z][a-z_]*\.[a-z][a-z_.]*\)".*/\1/p')
    if [ -n "$event_type" ]; then
        module=$(module_from_path "$file")
        while IFS= read -r subj; do
            [ -n "$subj" ] && PUBLISHED_SUBJECTS["$subj"]=1
        done < <(resolve_subjects "$module" "$event_type")
    fi
done < <(grep -rn 'enqueue_event' "$MODULES_DIR" --include="*.rs" -A2 \
    | grep '"[a-z][a-z_]*\.[a-z]' | grep -v '/tests/' || true)

# 2d. Self-published events (module publishes to subjects it also consumes)
while IFS= read -r line; do
    file=$(echo "$line" | cut -d: -f1)
    subject=$(echo "$line" | sed -n 's/.*publish("\([^"]*\)".*/\1/p')
    if [ -n "$subject" ]; then
        PUBLISHED_SUBJECTS["$subject"]=1
    fi
done < <(grep -rn '\.publish("' "$MODULES_DIR" --include="*.rs" | grep -v '/tests/' || true)

echo "Published subjects resolved: ${#PUBLISHED_SUBJECTS[@]}"
echo ""

# ── Step 3: Check SDK .consumer() subjects (STRICT — fails CI) ───

echo "--- SDK .consumer() contracts (strict) ---"

for subject in $(echo "${!SDK_CONSUMERS[@]}" | tr ' ' '\n' | sort); do
    source_file="${SDK_CONSUMERS[$subject]}"
    short_file="${source_file#$PROJECT_ROOT/}"

    if [ -n "${PUBLISHED_SUBJECTS[$subject]+x}" ]; then
        echo "  OK  $subject  ($short_file)"
    else
        echo "  FAIL  $subject  ($short_file) — no matching publisher"
        ERRORS=$((ERRORS + 1))
    fi
done

# ── Step 4: Check extended subscribe patterns (ADVISORY) ─────────

WARNINGS=0

if [ ${#EXTENDED_CONSUMERS[@]} -gt 0 ]; then
    echo ""
    echo "--- Extended subscribe patterns (advisory) ---"

    for subject in $(echo "${!EXTENDED_CONSUMERS[@]}" | tr ' ' '\n' | sort); do
        source_file="${EXTENDED_CONSUMERS[$subject]}"
        short_file="${source_file#$PROJECT_ROOT/}"

        if [ -n "${PUBLISHED_SUBJECTS[$subject]+x}" ]; then
            echo "  OK  $subject  ($short_file)"
        else
            echo "  WARN  $subject  ($short_file) — no matching publisher"
            WARNINGS=$((WARNINGS + 1))
        fi
    done
fi

echo ""
echo "──────────────────────────────────────────"
if [ "$ERRORS" -gt 0 ]; then
    echo "FAIL: $ERRORS SDK consumer subject(s) with no matching publisher"
    exit 1
else
    echo "OK: All ${#SDK_CONSUMERS[@]} SDK consumer subjects have matching publishers"
    if [ "$WARNINGS" -gt 0 ]; then
        echo "ADVISORY: $WARNINGS extended subject(s) without matching publisher"
    fi
    exit 0
fi
