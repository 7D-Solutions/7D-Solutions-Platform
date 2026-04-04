#!/usr/bin/env bash
# check-consumer-publisher-join.sh — Validate that every .consumer() subject
# resolves to a real publisher subject.
#
# Uses the event catalog (generated from source) as the ground truth for
# published subjects, then checks every .consumer() call in modules/*.
#
# Exit code: 0 if all consumers match a publisher, 1 otherwise.

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODULES_DIR="$PROJECT_ROOT/modules"
CATALOG="$PROJECT_ROOT/docs/event-catalog.md"

FAILURES=0
CHECKED=0

# ── Extract published subjects from event catalog ──────────────
# The catalog has rows like: | `event_type` | `nats.subject` | ...
declare -A PUBLISHED_SUBJECTS
if [ ! -f "$CATALOG" ]; then
  echo "ERROR: $CATALOG not found. Run ./scripts/generate-event-catalog.sh first."
  exit 1
fi

while IFS= read -r line; do
  subject=$(echo "$line" | sed -n 's/.*| `\([^`]*\)` |.*/\1/p')
  if [ -n "$subject" ]; then
    PUBLISHED_SUBJECTS["$subject"]=1
  fi
done < <(awk -F'|' '/^\|.*\`.*\.events\./ { print $3 }' "$CATALOG" | grep -oE '`[^`]+`' | tr -d '`' | while read -r s; do echo "| \`$s\` |"; done)

# Simpler extraction: grab the second backtick-quoted field from table rows
while IFS= read -r line; do
  # Table rows: | `event_type` | `nats_subject` | consumers | source |
  subject=$(echo "$line" | awk -F'`' '{ if (NF >= 4) print $4 }')
  if [ -n "$subject" ] && [[ "$subject" == *.* ]]; then
    PUBLISHED_SUBJECTS["$subject"]=1
  fi
done < <(grep '^|' "$CATALOG" | grep -v '^|--' | grep -v '^| Event')

# ── Extract consumer subjects from source ──────────────────────
# SDK .consumer("subject", handler) calls
while IFS= read -r line; do
  file=$(echo "$line" | cut -d: -f1)
  lineno=$(echo "$line" | cut -d: -f2)
  subject=$(echo "$line" | sed -n 's/.*\.consumer("\([^"]*\)".*/\1/p')
  if [ -z "$subject" ]; then
    continue
  fi

  CHECKED=$((CHECKED + 1))
  module=$(echo "$file" | sed -n "s|$MODULES_DIR/\([^/]*\)/.*|\1|p")

  if [ -z "${PUBLISHED_SUBJECTS[$subject]+x}" ]; then
    echo "FAIL: $module subscribes to '$subject' — no matching publisher"
    echo "      at $file:$lineno"
    FAILURES=$((FAILURES + 1))
  fi
done < <(grep -rn '\.consumer("' "$MODULES_DIR" --include="*.rs" | grep -v '/tests/' | grep -v '/target/')

# Also check multi-line .consumer( calls where subject is on the next line
while IFS= read -r line; do
  file=$(echo "$line" | cut -d: -f1)
  lineno=$(echo "$line" | cut -d: -f2)
  # Check if subject is on this line
  subject=$(echo "$line" | sed -n 's/.*\.consumer("\([^"]*\)".*/\1/p')
  if [ -n "$subject" ]; then
    continue  # Already handled above
  fi
  # Check next line for subject
  nextline_no=$((lineno + 1))
  subject=$(sed -n "${nextline_no}p" "$file" 2>/dev/null | sed -n 's/.*"\([^"]*\)".*/\1/p')
  if [ -z "$subject" ] || [[ "$subject" != *.* ]]; then
    continue
  fi

  CHECKED=$((CHECKED + 1))
  module=$(echo "$file" | sed -n "s|$MODULES_DIR/\([^/]*\)/.*|\1|p")

  if [ -z "${PUBLISHED_SUBJECTS[$subject]+x}" ]; then
    echo "FAIL: $module subscribes to '$subject' — no matching publisher"
    echo "      at $file:$nextline_no"
    FAILURES=$((FAILURES + 1))
  fi
done < <(grep -rn '\.consumer(' "$MODULES_DIR" --include="*.rs" | grep -v '/tests/' | grep -v '/target/' | grep -v '\.consumer("')

# ── Also index self-published subjects (inline .bind / enqueue in same module) ──
while IFS= read -r line; do
  subject=$(echo "$line" | sed -n 's/.*"\([a-z][a-z0-9_]*\.[a-z][a-z0-9_.]*\)".*/\1/p')
  if [ -n "$subject" ]; then
    PUBLISHED_SUBJECTS["$subject"]=1
  fi
done < <(grep -rn '\.bind(".*\.' "$MODULES_DIR" --include="*.rs" | grep -v '/tests/' | grep -v '/target/' || true)

# ── Also check module.toml [events.subscribe] subjects ─────────
while IFS= read -r toml_file; do
  module=$(echo "$toml_file" | sed -n "s|$MODULES_DIR/\([^/]*\)/.*|\1|p")
  # Extract subjects from subjects = ["..."]
  while IFS= read -r subject; do
    [ -z "$subject" ] && continue
    CHECKED=$((CHECKED + 1))
    if [ -z "${PUBLISHED_SUBJECTS[$subject]+x}" ]; then
      echo "FAIL: $module module.toml subscribes to '$subject' — no matching publisher"
      echo "      at $toml_file"
      FAILURES=$((FAILURES + 1))
    fi
  done < <(grep 'subjects' "$toml_file" 2>/dev/null | grep -oE '"[^"]*"' | tr -d '"')
done < <(find "$MODULES_DIR" -name "module.toml" -type f 2>/dev/null)

# ── Report ─────────────────────────────────────────────────────
echo ""
echo "Consumer-publisher join: $CHECKED checked, $FAILURES failures"

if [ "$FAILURES" -gt 0 ]; then
  exit 1
fi

echo "All consumer subjects resolve to known publishers."
exit 0
