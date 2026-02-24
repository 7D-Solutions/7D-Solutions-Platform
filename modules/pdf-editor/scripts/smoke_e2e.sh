#!/usr/bin/env bash
# smoke_e2e.sh — End-to-end smoke test for the pdf-editor module.
#
# Tests the full backend flow: health, PDF rendering, forms CRUD,
# submissions flow, PDF generation, and NATS event delivery.
#
# Requirements: curl, jq, psql (or PSQL env var pointing to the binary)
# Usage:
#   BASE_URL=http://localhost:8102 \
#   PSQL_URL=postgresql://pdf_editor_user:pdf_editor_pass@localhost:5453/pdf_editor_db \
#   modules/pdf-editor/scripts/smoke_e2e.sh
#
# NATS events are verified via the events_outbox table (status='published').
# The outbox publisher polls every 100ms; a 3-second grace window is used.

set -euo pipefail

BASE_URL="${BASE_URL:-http://localhost:8102}"
PSQL_URL="${PSQL_URL:-postgresql://pdf_editor_user:pdf_editor_pass@localhost:5453/pdf_editor_db}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIXTURE_DIR="$SCRIPT_DIR/../fixtures"

SAMPLE_PDF="$FIXTURE_DIR/sample.pdf"
SAMPLE_ANNOTATIONS="$FIXTURE_DIR/sample_annotations.json"

# Unique tenant per run to avoid cross-test interference.
TENANT_ID="smoke-$(date +%s)-$$"

# Locate psql — accepts env override or searches common paths.
find_psql() {
    if [[ -n "${PSQL:-}" ]]; then echo "$PSQL"; return; fi
    for p in psql \
              /opt/homebrew/Cellar/libpq/*/bin/psql \
              /opt/homebrew/bin/psql \
              /usr/bin/psql \
              /usr/local/bin/psql; do
        # expand glob if needed
        for resolved in $p; do
            if command -v "$resolved" &>/dev/null || [[ -x "$resolved" ]]; then
                echo "$resolved"
                return
            fi
        done
    done
    echo ""
}

PSQL_BIN="$(find_psql)"

# ─── helpers ────────────────────────────────────────────────────────────────

PASS=0
FAIL=0

step() { echo; echo "▶ $*"; }
ok()   { echo "  ✓ $*"; PASS=$((PASS + 1)); }
fail() { echo "  ✗ $*"; FAIL=$((FAIL + 1)); }

assert_http() {
    local label="$1" expected="$2" actual="$3"
    if [[ "$actual" == "$expected" ]]; then
        ok "$label (HTTP $actual)"
    else
        fail "$label — expected HTTP $expected, got HTTP $actual"
        return 1
    fi
}

assert_pdf_magic() {
    local label="$1" file="$2"
    local magic
    magic=$(head -c 4 "$file" 2>/dev/null || true)
    if [[ "$magic" == "%PDF" ]]; then
        ok "$label — response is valid PDF (%PDF-)"
    else
        fail "$label — response does not start with %PDF- (got: $magic)"
        return 1
    fi
}

assert_json_field() {
    local label="$1" json="$2" field="$3" expected="$4"
    local actual
    actual=$(echo "$json" | jq -r "$field" 2>/dev/null)
    if [[ "$actual" == "$expected" ]]; then
        ok "$label ($field = $actual)"
    else
        fail "$label — $field: expected '$expected', got '$actual'"
        return 1
    fi
}

psql_query() {
    if [[ -z "$PSQL_BIN" ]]; then
        echo "SKIP_NO_PSQL"
        return
    fi
    "$PSQL_BIN" "$PSQL_URL" -t -c "$1" 2>/dev/null | tr -d '[:space:]'
}

wait_for_event() {
    local subject="$1" tenant="$2" max_wait=5 elapsed=0
    while (( elapsed < max_wait )); do
        local count
        count=$(psql_query "SELECT COUNT(*) FROM events_outbox WHERE subject='$subject' AND tenant_id='$tenant' AND status='published';")
        if [[ "$count" =~ ^[1-9] ]]; then
            echo "$count"
            return 0
        fi
        sleep 0.5
        (( elapsed++ )) || true
    done
    echo "0"
}

# ─── pre-flight ─────────────────────────────────────────────────────────────

echo "=================================================="
echo " pdf-editor E2E smoke test"
echo " BASE_URL:  $BASE_URL"
echo " PSQL_URL:  $PSQL_URL"
echo " TENANT_ID: $TENANT_ID"
echo "=================================================="

for tool in curl jq; do
    if ! command -v "$tool" &>/dev/null; then
        echo "ERROR: $tool is required but not installed." >&2
        exit 1
    fi
done

if [[ ! -f "$SAMPLE_PDF" ]]; then
    echo "ERROR: fixture not found: $SAMPLE_PDF" >&2
    exit 1
fi
if [[ ! -f "$SAMPLE_ANNOTATIONS" ]]; then
    echo "ERROR: fixture not found: $SAMPLE_ANNOTATIONS" >&2
    exit 1
fi

if [[ -z "$PSQL_BIN" ]]; then
    echo "  ⚠ psql not found — NATS event checks will be skipped"
fi

# ─── step 1: health ─────────────────────────────────────────────────────────

step "Health check"

HEALTH_CODE=$(curl -s -o /tmp/smoke_health.json -w "%{http_code}" "$BASE_URL/api/health")
assert_http "GET /api/health" 200 "$HEALTH_CODE"
HEALTH_JSON=$(cat /tmp/smoke_health.json)
assert_json_field "health.status" "$HEALTH_JSON" '.status' "healthy"

# ─── step 2: render-annotations (stateless) ─────────────────────────────────

step "POST /api/pdf/render-annotations"

RENDER_CODE=$(curl -s \
    -F "file=@${SAMPLE_PDF};type=application/pdf" \
    -F "annotations=<${SAMPLE_ANNOTATIONS};type=application/json" \
    -o /tmp/smoke_rendered.pdf \
    -w "%{http_code}" \
    "$BASE_URL/api/pdf/render-annotations")
assert_http "POST /api/pdf/render-annotations" 200 "$RENDER_CODE"
assert_pdf_magic "render-annotations response" /tmp/smoke_rendered.pdf

# ─── step 3: form templates CRUD ────────────────────────────────────────────

step "Form templates CRUD"

# Create
CREATE_TMPL_CODE=$(curl -s \
    -X POST "$BASE_URL/api/pdf/forms/templates" \
    -H "Content-Type: application/json" \
    -d "{\"tenant_id\":\"$TENANT_ID\",\"name\":\"Smoke Test Form\",\"description\":\"Created by smoke_e2e.sh\",\"created_by\":\"smoke-user\"}" \
    -o /tmp/smoke_tmpl.json \
    -w "%{http_code}")
assert_http "POST /api/pdf/forms/templates" 201 "$CREATE_TMPL_CODE"
TMPL_JSON=$(cat /tmp/smoke_tmpl.json)
TMPL_ID=$(echo "$TMPL_JSON" | jq -r '.id')
assert_json_field "template.name" "$TMPL_JSON" '.name' "Smoke Test Form"
assert_json_field "template.tenant_id" "$TMPL_JSON" '.tenant_id' "$TENANT_ID"

# List
LIST_TMPL_CODE=$(curl -s \
    "$BASE_URL/api/pdf/forms/templates?tenant_id=$TENANT_ID" \
    -o /tmp/smoke_tmpls.json \
    -w "%{http_code}")
assert_http "GET /api/pdf/forms/templates" 200 "$LIST_TMPL_CODE"
LIST_COUNT=$(cat /tmp/smoke_tmpls.json | jq 'length')
if [[ "$LIST_COUNT" -ge 1 ]]; then
    ok "list templates — found $LIST_COUNT template(s)"
else
    fail "list templates — expected >= 1, got $LIST_COUNT"
fi

# Get by ID
GET_TMPL_CODE=$(curl -s \
    "$BASE_URL/api/pdf/forms/templates/$TMPL_ID?tenant_id=$TENANT_ID" \
    -o /tmp/smoke_tmpl_get.json \
    -w "%{http_code}")
assert_http "GET /api/pdf/forms/templates/:id" 200 "$GET_TMPL_CODE"
assert_json_field "get template.id" "$(cat /tmp/smoke_tmpl_get.json)" '.id' "$TMPL_ID"

# Update
UPDATE_TMPL_CODE=$(curl -s \
    -X PUT "$BASE_URL/api/pdf/forms/templates/$TMPL_ID?tenant_id=$TENANT_ID" \
    -H "Content-Type: application/json" \
    -d '{"name":"Smoke Test Form (updated)"}' \
    -o /tmp/smoke_tmpl_upd.json \
    -w "%{http_code}")
assert_http "PUT /api/pdf/forms/templates/:id" 200 "$UPDATE_TMPL_CODE"
assert_json_field "updated template.name" "$(cat /tmp/smoke_tmpl_upd.json)" '.name' "Smoke Test Form (updated)"

# ─── step 4: form fields CRUD ───────────────────────────────────────────────

step "Form fields CRUD"

# Create field 1
CREATE_F1_CODE=$(curl -s \
    -X POST "$BASE_URL/api/pdf/forms/templates/$TMPL_ID/fields?tenant_id=$TENANT_ID" \
    -H "Content-Type: application/json" \
    -d '{"field_key":"inspector_name","field_label":"Inspector Name","field_type":"text","validation_rules":{"required":true},"pdf_position":{"x":100,"y":700,"page":1,"font_size":14}}' \
    -o /tmp/smoke_f1.json \
    -w "%{http_code}")
assert_http "POST /api/pdf/forms/templates/:id/fields (field 1)" 201 "$CREATE_F1_CODE"
FIELD1_ID=$(cat /tmp/smoke_f1.json | jq -r '.id')
assert_json_field "field1.field_key" "$(cat /tmp/smoke_f1.json)" '.field_key' "inspector_name"

# Create field 2
CREATE_F2_CODE=$(curl -s \
    -X POST "$BASE_URL/api/pdf/forms/templates/$TMPL_ID/fields?tenant_id=$TENANT_ID" \
    -H "Content-Type: application/json" \
    -d '{"field_key":"passed","field_label":"Passed","field_type":"checkbox","validation_rules":{"required":true},"pdf_position":{"x":100,"y":660,"page":1,"font_size":12}}' \
    -o /tmp/smoke_f2.json \
    -w "%{http_code}")
assert_http "POST /api/pdf/forms/templates/:id/fields (field 2)" 201 "$CREATE_F2_CODE"
FIELD2_ID=$(cat /tmp/smoke_f2.json | jq -r '.id')

# List fields
LIST_FIELDS_CODE=$(curl -s \
    "$BASE_URL/api/pdf/forms/templates/$TMPL_ID/fields?tenant_id=$TENANT_ID" \
    -o /tmp/smoke_fields.json \
    -w "%{http_code}")
assert_http "GET /api/pdf/forms/templates/:id/fields" 200 "$LIST_FIELDS_CODE"
FIELD_COUNT=$(cat /tmp/smoke_fields.json | jq 'length')
if [[ "$FIELD_COUNT" -eq 2 ]]; then
    ok "list fields — found 2 fields"
else
    fail "list fields — expected 2, got $FIELD_COUNT"
fi

# Update field
UPDATE_F1_CODE=$(curl -s \
    -X PUT "$BASE_URL/api/pdf/forms/templates/$TMPL_ID/fields/$FIELD1_ID?tenant_id=$TENANT_ID" \
    -H "Content-Type: application/json" \
    -d '{"field_label":"Lead Inspector Name"}' \
    -o /tmp/smoke_f1_upd.json \
    -w "%{http_code}")
assert_http "PUT /api/pdf/forms/templates/:tid/fields/:fid" 200 "$UPDATE_F1_CODE"
assert_json_field "updated field.field_label" "$(cat /tmp/smoke_f1_upd.json)" '.field_label' "Lead Inspector Name"

# Reorder fields
REORDER_CODE=$(curl -s \
    -X POST "$BASE_URL/api/pdf/forms/templates/$TMPL_ID/fields/reorder?tenant_id=$TENANT_ID" \
    -H "Content-Type: application/json" \
    -d "{\"field_ids\":[\"$FIELD2_ID\",\"$FIELD1_ID\"]}" \
    -o /tmp/smoke_reorder.json \
    -w "%{http_code}")
assert_http "POST .../fields/reorder" 200 "$REORDER_CODE"
FIRST_AFTER_REORDER=$(cat /tmp/smoke_reorder.json | jq -r '.[0].id')
if [[ "$FIRST_AFTER_REORDER" == "$FIELD2_ID" ]]; then
    ok "reorder fields — order confirmed"
else
    fail "reorder fields — expected field2 first, got $FIRST_AFTER_REORDER"
fi

# ─── step 5: submissions flow ───────────────────────────────────────────────

step "Submissions flow"

# Create draft
CREATE_SUB_CODE=$(curl -s \
    -X POST "$BASE_URL/api/pdf/forms/submissions" \
    -H "Content-Type: application/json" \
    -d "{\"tenant_id\":\"$TENANT_ID\",\"template_id\":\"$TMPL_ID\",\"submitted_by\":\"smoke-worker\"}" \
    -o /tmp/smoke_sub.json \
    -w "%{http_code}")
assert_http "POST /api/pdf/forms/submissions (create draft)" 201 "$CREATE_SUB_CODE"
SUB_JSON=$(cat /tmp/smoke_sub.json)
SUB_ID=$(echo "$SUB_JSON" | jq -r '.id')
assert_json_field "submission.status" "$SUB_JSON" '.status' "draft"

# Autosave
AUTOSAVE_CODE=$(curl -s \
    -X PUT "$BASE_URL/api/pdf/forms/submissions/$SUB_ID?tenant_id=$TENANT_ID" \
    -H "Content-Type: application/json" \
    -d '{"field_data":{"inspector_name":"Jane Smith","passed":true}}' \
    -o /tmp/smoke_autosave.json \
    -w "%{http_code}")
assert_http "PUT /api/pdf/forms/submissions/:id (autosave)" 200 "$AUTOSAVE_CODE"
AUTOSAVE_JSON=$(cat /tmp/smoke_autosave.json)
assert_json_field "autosave.status" "$AUTOSAVE_JSON" '.status' "draft"
AUTOSAVE_NAME=$(echo "$AUTOSAVE_JSON" | jq -r '.field_data.inspector_name')
if [[ "$AUTOSAVE_NAME" == "Jane Smith" ]]; then
    ok "autosave — field_data persisted"
else
    fail "autosave — inspector_name: expected 'Jane Smith', got '$AUTOSAVE_NAME'"
fi

# Get submission
GET_SUB_CODE=$(curl -s \
    "$BASE_URL/api/pdf/forms/submissions/$SUB_ID?tenant_id=$TENANT_ID" \
    -o /tmp/smoke_sub_get.json \
    -w "%{http_code}")
assert_http "GET /api/pdf/forms/submissions/:id" 200 "$GET_SUB_CODE"
assert_json_field "get submission.id" "$(cat /tmp/smoke_sub_get.json)" '.id' "$SUB_ID"

# Submit
SUBMIT_CODE=$(curl -s \
    -X POST "$BASE_URL/api/pdf/forms/submissions/$SUB_ID/submit?tenant_id=$TENANT_ID" \
    -o /tmp/smoke_submit.json \
    -w "%{http_code}")
assert_http "POST /api/pdf/forms/submissions/:id/submit" 200 "$SUBMIT_CODE"
assert_json_field "submitted.status" "$(cat /tmp/smoke_submit.json)" '.status' "submitted"

# ─── step 6: PDF generation ─────────────────────────────────────────────────

step "PDF generation"

GENERATE_CODE=$(curl -s \
    -X POST "$BASE_URL/api/pdf/forms/submissions/$SUB_ID/generate?tenant_id=$TENANT_ID" \
    -F "file=@${SAMPLE_PDF};type=application/pdf" \
    -o /tmp/smoke_generated.pdf \
    -w "%{http_code}")
assert_http "POST .../generate" 200 "$GENERATE_CODE"
assert_pdf_magic "generate PDF response" /tmp/smoke_generated.pdf

# ─── step 7: NATS events via outbox ─────────────────────────────────────────

step "NATS events (via events_outbox)"

if [[ -z "$PSQL_BIN" ]]; then
    echo "  ⚠ psql not available — skipping event verification"
else
    # Allow up to 5 seconds for the outbox publisher (polls every 100ms)
    SUBMITTED_COUNT=$(wait_for_event "pdf.form.submitted" "$TENANT_ID")
    if [[ "$SUBMITTED_COUNT" =~ ^[1-9] ]]; then
        ok "pdf.form.submitted published to NATS ($SUBMITTED_COUNT event(s))"
    else
        fail "pdf.form.submitted — not observed as published within 5s"
    fi

    GENERATED_COUNT=$(wait_for_event "pdf.form.generated" "$TENANT_ID")
    if [[ "$GENERATED_COUNT" =~ ^[1-9] ]]; then
        ok "pdf.form.generated published to NATS ($GENERATED_COUNT event(s))"
    else
        fail "pdf.form.generated — not observed as published within 5s"
    fi
fi

# ─── summary ────────────────────────────────────────────────────────────────

echo
echo "=================================================="
echo " Results: $PASS passed, $FAIL failed"
echo "=================================================="

if [[ $FAIL -gt 0 ]]; then
    exit 1
fi
