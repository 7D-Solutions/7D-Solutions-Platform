#!/usr/bin/env bash
# Generates docs/plans/roadmap.html from MANUFACTURING-ROADMAP.md
# Run at milestones (not every bead). Agents include this in acceptance criteria.
# Usage: ./scripts/update-roadmap-html.sh

set -euo pipefail
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ROADMAP="$PROJECT_ROOT/docs/plans/MANUFACTURING-ROADMAP.md"
EXTRACTION="$PROJECT_ROOT/docs/FIREPROOF-EXTRACTION-PLAN.md"
OUTPUT="$PROJECT_ROOT/docs/plans/roadmap.html"

if [[ ! -f "$ROADMAP" ]]; then
  echo "ERROR: $ROADMAP not found" >&2
  exit 1
fi

# Parse phases and deliverables from the markdown
# Each deliverable line looks like: | text | STATUS | bd-xxx | date |
parse_deliverables() {
  local phase_id="$1"
  local in_phase=false
  local in_table=false
  local header_seen=false

  while IFS= read -r line; do
    # Detect phase heading
    if [[ "$line" =~ ^##[[:space:]]Phase[[:space:]]"$phase_id" ]] || \
       [[ "$phase_id" == "Pre-B" && "$line" =~ "Pre-B Infrastructure" ]]; then
      in_phase=true
      continue
    fi
    # Exit phase on next ## heading
    if $in_phase && [[ "$line" =~ ^##[[:space:]] ]] && ! [[ "$line" =~ "Phase $phase_id" ]]; then
      break
    fi
    if ! $in_phase; then continue; fi

    # Detect deliverable table (starts with | Deliverable |)
    if [[ "$line" =~ ^\|[[:space:]]*Deliverable ]]; then
      in_table=true
      header_seen=false
      continue
    fi
    # Skip separator line
    if $in_table && [[ "$line" =~ ^\|[-[:space:]\|]+$ ]]; then
      header_seen=true
      continue
    fi
    # End of table
    if $in_table && $header_seen && ! [[ "$line" =~ ^\| ]]; then
      break
    fi
    # Parse deliverable row
    if $in_table && $header_seen && [[ "$line" =~ ^\| ]]; then
      local deliverable status bead date
      deliverable=$(echo "$line" | awk -F'|' '{gsub(/^[[:space:]]+|[[:space:]]+$/,"",$2); print $2}')
      status=$(echo "$line" | awk -F'|' '{gsub(/^[[:space:]]+|[[:space:]]+$/,"",$3); print $3}')
      bead=$(echo "$line" | awk -F'|' '{gsub(/^[[:space:]]+|[[:space:]]+$/,"",$4); print $4}')
      date=$(echo "$line" | awk -F'|' '{gsub(/^[[:space:]]+|[[:space:]]+$/,"",$5); print $5}')
      # Escape HTML
      deliverable=$(echo "$deliverable" | sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g; s/`//g')
      echo "DELIV|${status}|${bead}|${date}|${deliverable}"
    fi
  done < "$ROADMAP"
}

# Count stats
count_status() {
  local phase_id="$1"
  local target="$2"
  parse_deliverables "$phase_id" | grep -c "^DELIV|${target}|" || true
}

# Generate phase status
phase_status() {
  local phase_id="$1"
  local total done not_started
  total=$(parse_deliverables "$phase_id" | wc -l | tr -d ' ')
  done=$(count_status "$phase_id" "DONE")
  not_started=$(count_status "$phase_id" "NOT STARTED")

  if [[ "$total" -eq 0 ]]; then
    echo "empty"
  elif [[ "$done" -eq "$total" ]]; then
    echo "complete"
  elif [[ "$not_started" -eq "$total" ]]; then
    echo "not-started"
  else
    echo "in-progress"
  fi
}

# Get last updated timestamp from the roadmap
last_updated=$(grep -m1 "^\*\*Last Updated:\*\*" "$ROADMAP" | sed 's/\*\*Last Updated:\*\*[[:space:]]*//' || echo "unknown")

# Build the extraction tier 1 items
extraction_items=""
if [[ -f "$EXTRACTION" ]]; then
  extraction_items=$(grep -A1 "^### Tier 1" "$EXTRACTION" | tail -1 || true)
fi

# Start generating HTML
cat > "$OUTPUT" << 'HTMLHEAD'
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta http-equiv="refresh" content="30">
<title>Manufacturing Roadmap</title>
<style>
  :root {
    --bg: #0d1117;
    --surface: #161b22;
    --border: #30363d;
    --text: #e6edf3;
    --text-dim: #8b949e;
    --green: #3fb950;
    --green-bg: #0d2818;
    --green-border: #1b4332;
    --yellow: #d29922;
    --yellow-bg: #2d1f00;
    --yellow-border: #4a3300;
    --red: #f85149;
    --blue: #58a6ff;
    --blue-bg: #0c2d6b;
    --blue-border: #1a4b8f;
    --purple: #bc8cff;
  }
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body {
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif;
    background: var(--bg);
    color: var(--text);
    padding: 24px;
    line-height: 1.5;
  }
  .header {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 32px;
    border-bottom: 1px solid var(--border);
    padding-bottom: 16px;
  }
  .header h1 { font-size: 24px; font-weight: 600; }
  .header .meta { color: var(--text-dim); font-size: 13px; }
  .refresh-note { color: var(--text-dim); font-size: 11px; font-style: italic; }

  .summary {
    display: flex;
    gap: 12px;
    margin-bottom: 32px;
    flex-wrap: wrap;
  }
  .summary-card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 16px 20px;
    min-width: 140px;
    flex: 1;
  }
  .summary-card .label { font-size: 12px; color: var(--text-dim); text-transform: uppercase; letter-spacing: 0.5px; }
  .summary-card .value { font-size: 28px; font-weight: 600; margin-top: 4px; }
  .summary-card.done .value { color: var(--green); }
  .summary-card.progress .value { color: var(--yellow); }
  .summary-card.pending .value { color: var(--text-dim); }
  .summary-card.total .value { color: var(--blue); }

  .dep-chain {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 16px 20px;
    margin-bottom: 32px;
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
    font-size: 14px;
  }
  .dep-chain .arrow { color: var(--text-dim); font-size: 18px; }
  .dep-chip {
    padding: 4px 12px;
    border-radius: 16px;
    font-weight: 500;
    font-size: 13px;
    white-space: nowrap;
  }
  .dep-chip.complete { background: var(--green-bg); border: 1px solid var(--green-border); color: var(--green); }
  .dep-chip.in-progress { background: var(--yellow-bg); border: 1px solid var(--yellow-border); color: var(--yellow); }
  .dep-chip.not-started { background: var(--surface); border: 1px solid var(--border); color: var(--text-dim); }

  .phase {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    margin-bottom: 16px;
    overflow: hidden;
  }
  .phase-header {
    padding: 14px 20px;
    display: flex;
    justify-content: space-between;
    align-items: center;
    cursor: pointer;
    user-select: none;
  }
  .phase-header:hover { background: rgba(255,255,255,0.03); }
  .phase-title { font-weight: 600; font-size: 16px; }
  .phase-goal { color: var(--text-dim); font-size: 13px; margin-left: 12px; font-weight: 400; }
  .phase-badge {
    font-size: 12px;
    padding: 3px 10px;
    border-radius: 12px;
    font-weight: 500;
    white-space: nowrap;
  }
  .badge-complete { background: var(--green-bg); border: 1px solid var(--green-border); color: var(--green); }
  .badge-in-progress { background: var(--yellow-bg); border: 1px solid var(--yellow-border); color: var(--yellow); }
  .badge-not-started { background: var(--surface); border: 1px solid var(--border); color: var(--text-dim); }

  .phase-progress {
    height: 3px;
    background: var(--border);
  }
  .phase-progress-bar {
    height: 100%;
    transition: width 0.3s ease;
  }
  .phase-progress-bar.complete { background: var(--green); }
  .phase-progress-bar.partial { background: var(--yellow); }

  .phase-body { padding: 0; display: none; }
  .phase.open .phase-body { display: block; }
  .phase.open .phase-header { border-bottom: 1px solid var(--border); }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  th {
    text-align: left;
    padding: 10px 16px;
    color: var(--text-dim);
    font-weight: 500;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.3px;
    border-bottom: 1px solid var(--border);
  }
  td {
    padding: 10px 16px;
    border-bottom: 1px solid rgba(48,54,61,0.5);
    vertical-align: top;
  }
  tr:last-child td { border-bottom: none; }
  tr:hover td { background: rgba(255,255,255,0.02); }

  .status-done { color: var(--green); font-weight: 500; }
  .status-not-started { color: var(--text-dim); }
  .status-in-progress { color: var(--yellow); font-weight: 500; }
  .bead-link { color: var(--purple); font-family: monospace; font-size: 12px; }
  .date-cell { color: var(--text-dim); font-size: 12px; white-space: nowrap; }
  .deliverable-cell { max-width: 500px; }

  .extraction-section {
    background: var(--blue-bg);
    border: 1px solid var(--blue-border);
    border-radius: 8px;
    padding: 16px 20px;
    margin-bottom: 16px;
  }
  .extraction-section h3 { color: var(--blue); font-size: 14px; margin-bottom: 8px; }
  .extraction-section .item { padding: 6px 0; font-size: 13px; }
  .extraction-section .item-status { font-weight: 500; }

  .scope-fences {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 16px 20px;
    margin-bottom: 32px;
    font-size: 13px;
    color: var(--text-dim);
  }
  .scope-fences h3 { color: var(--text); font-size: 14px; margin-bottom: 8px; }
  .scope-fences ul { padding-left: 20px; }
  .scope-fences li { margin-bottom: 4px; }

  .toggle-icon { transition: transform 0.2s; display: inline-block; }
  .phase.open .toggle-icon { transform: rotate(90deg); }

  @media (max-width: 768px) {
    .summary { flex-direction: column; }
    .dep-chain { flex-direction: column; align-items: flex-start; }
  }
</style>
</head>
<body>
HTMLHEAD

# Write header
cat >> "$OUTPUT" << EOF
<div class="header">
  <div>
    <h1>Manufacturing Roadmap</h1>
    <span class="refresh-note">Auto-refreshes every 30s &middot; Last generated: $(date '+%Y-%m-%d %H:%M')</span>
  </div>
  <div class="meta">Roadmap updated: ${last_updated}</div>
</div>
EOF

# Calculate totals across all phases
phases=("0" "A" "B" "C1" "C2" "D" "E")
phase_names=("Design Lock" "Inventory + BOM" "Production Execution" "Receiving Inspection" "In-Process + Final Inspection" "ECO + Change Control" "Maintenance Workcenter")
phase_goals=("Prevent one-way-door mistakes" "Product structure + inventory movements" "WO → issue → execute → receipt" "Incoming material inspection" "Production-integrated inspection" "Change control evidence" "Production ↔ Maintenance loop")

total_all=0
done_all=0
progress_all=0
not_started_all=0

for pid in "${phases[@]}"; do
  t=$(parse_deliverables "$pid" | wc -l | tr -d ' ')
  d=$(count_status "$pid" "DONE")
  n=$(count_status "$pid" "NOT STARTED")
  total_all=$((total_all + t))
  done_all=$((done_all + d))
  not_started_all=$((not_started_all + n))
done
progress_all=$((total_all - done_all - not_started_all))

# Summary cards
cat >> "$OUTPUT" << EOF
<div class="summary">
  <div class="summary-card total"><div class="label">Total Deliverables</div><div class="value">${total_all}</div></div>
  <div class="summary-card done"><div class="label">Complete</div><div class="value">${done_all}</div></div>
  <div class="summary-card progress"><div class="label">In Progress</div><div class="value">${progress_all}</div></div>
  <div class="summary-card pending"><div class="label">Not Started</div><div class="value">${not_started_all}</div></div>
</div>
EOF

# Dependency chain
echo '<div class="dep-chain">' >> "$OUTPUT"
echo '<strong style="margin-right:4px">Dependency:</strong>' >> "$OUTPUT"
dep_phases=("0" "A" "B" "C2")
dep_labels=("Phase 0" "Phase A" "Phase B" "Phase C2")
for i in "${!dep_phases[@]}"; do
  pid="${dep_phases[$i]}"
  lbl="${dep_labels[$i]}"
  st=$(phase_status "$pid")
  echo "<span class=\"dep-chip ${st}\">${lbl}</span>" >> "$OUTPUT"
  if [[ $i -lt $((${#dep_phases[@]} - 1)) ]]; then
    echo '<span class="arrow">→</span>' >> "$OUTPUT"
  fi
done
echo '<span class="arrow" style="margin-left:12px">|</span>' >> "$OUTPUT"
# Parallel tracks
for pid_lbl in "C1:Phase C1" "D:Phase D" "E:Phase E"; do
  pid="${pid_lbl%%:*}"
  lbl="${pid_lbl##*:}"
  st=$(phase_status "$pid")
  echo "<span class=\"dep-chip ${st}\">${lbl}</span>" >> "$OUTPUT"
done
echo '</div>' >> "$OUTPUT"

# Extraction section
cat >> "$OUTPUT" << 'EOF'
<div class="extraction-section">
  <h3>Pre-B Infrastructure (Fireproof Extraction)</h3>
EOF

# Check if event-consumer crate exists
if [[ -d "$PROJECT_ROOT/platform/event-consumer" ]]; then
  echo '  <div class="item"><span class="status-done item-status">DONE</span> — Event consumer crate (platform/event-consumer/)</div>' >> "$OUTPUT"
else
  echo '  <div class="item"><span class="status-not-started item-status">NOT STARTED</span> — Event consumer crate (platform/event-consumer/)</div>' >> "$OUTPUT"
fi

# Check if security_event.rs exists
if [[ -f "$PROJECT_ROOT/platform/security/src/security_event.rs" ]]; then
  echo '  <div class="item"><span class="status-done item-status">DONE</span> — Security audit log (platform/security/src/security_event.rs)</div>' >> "$OUTPUT"
else
  echo '  <div class="item"><span class="status-not-started item-status">NOT STARTED</span> — Security audit log (platform/security/src/security_event.rs)</div>' >> "$OUTPUT"
fi

echo '</div>' >> "$OUTPUT"

# Render each phase
for i in "${!phases[@]}"; do
  pid="${phases[$i]}"
  pname="${phase_names[$i]}"
  pgoal="${phase_goals[$i]}"
  st=$(phase_status "$pid")

  total=$(parse_deliverables "$pid" | wc -l | tr -d ' ')
  done_count=$(count_status "$pid" "DONE")

  if [[ "$total" -eq 0 ]]; then
    pct=0
  else
    pct=$(( (done_count * 100) / total ))
  fi

  # Badge class
  case "$st" in
    complete) badge_class="badge-complete" badge_text="COMPLETE" bar_class="complete" open_class="" ;;
    in-progress) badge_class="badge-in-progress" badge_text="IN PROGRESS" bar_class="partial" open_class="open" ;;
    *) badge_class="badge-not-started" badge_text="NOT STARTED" bar_class="" open_class="" ;;
  esac

  cat >> "$OUTPUT" << EOF
<div class="phase ${open_class}" onclick="this.classList.toggle('open')">
  <div class="phase-header">
    <div>
      <span class="toggle-icon">&#9654;</span>
      <span class="phase-title">Phase ${pid}</span>
      <span class="phase-goal">${pname} — ${pgoal}</span>
    </div>
    <div style="display:flex;align-items:center;gap:12px">
      <span style="color:var(--text-dim);font-size:12px">${done_count}/${total}</span>
      <span class="phase-badge ${badge_class}">${badge_text}</span>
    </div>
  </div>
  <div class="phase-progress"><div class="phase-progress-bar ${bar_class}" style="width:${pct}%"></div></div>
  <div class="phase-body">
    <table>
      <thead><tr><th>Deliverable</th><th>Status</th><th>Bead</th><th>Date</th></tr></thead>
      <tbody>
EOF

  # Render deliverable rows
  parse_deliverables "$pid" | while IFS='|' read -r _ status bead date deliverable; do
    case "$status" in
      DONE) status_class="status-done" ;;
      "NOT STARTED") status_class="status-not-started" ;;
      *) status_class="status-in-progress" ;;
    esac

    bead_display="—"
    if [[ "$bead" != "—" && -n "$bead" ]]; then
      bead_display="<span class=\"bead-link\">${bead}</span>"
    fi

    date_display="—"
    if [[ "$date" != "—" && -n "$date" ]]; then
      date_display="${date}"
    fi

    cat >> "$OUTPUT" << EOF
        <tr>
          <td class="deliverable-cell">${deliverable}</td>
          <td><span class="${status_class}">${status}</span></td>
          <td>${bead_display}</td>
          <td class="date-cell">${date_display}</td>
        </tr>
EOF
  done

  cat >> "$OUTPUT" << 'EOF'
      </tbody>
    </table>
  </div>
</div>
EOF

done

# Scope fences
cat >> "$OUTPUT" << 'EOF'
<div class="scope-fences">
  <h3>Scope Fences (Permanent)</h3>
  <ul>
    <li>Discrete manufacturing only — no process/recipe BOM</li>
    <li>No backflush in v1 — explicit component issue only</li>
    <li>No MRP/Planning — manual work order creation</li>
    <li>No NCR/CAPA lifecycle — hold/release only</li>
    <li>No production scheduling/capacity optimization</li>
    <li>Tests are integrated — real Postgres, real NATS, no mocks</li>
  </ul>
</div>
EOF

# Close HTML
cat >> "$OUTPUT" << 'EOF'
<script>
// Allow clicking phase headers to expand/collapse
// Already handled via onclick on .phase div
</script>
</body>
</html>
EOF

echo "Roadmap HTML generated: $OUTPUT"
echo "Open in browser: file://$OUTPUT"
