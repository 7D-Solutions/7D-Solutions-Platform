# Customer-Complaints Module — Scope, Boundaries, Contracts (draft v0.1)

**7D Solutions Platform**
**Status:** Draft Specification — bd-ixnbs migration
**Date:** 2026-04-16
**Proposed module name:** `customer-complaints`
**Source of migration:** Fireproof ERP (`customer_satisfaction/` module — the `FeedbackRecord` portion; the CSAT/NPS metric tracking stays in Fireproof per user ruling)
**Cross-vertical applicability:** Every B2B or B2C vertical — Fireproof (customer concern letters), HuberPower (outage/billing complaints), TrashTech (missed pickups, damaged bins, billing), RanchOrbit (livestock quality, service calls)

---

## 1. Mission & Non-Goals

### Mission
The Customer-Complaints module is the **authoritative system for inbound customer complaints from intake to resolution**: capturing who complained about what, routing for investigation, tracking action taken, confirming customer follow-up, and closing with an outcome.

### Non-Goals
Customer-Complaints does **NOT**:
- Own customer/party records (delegated to Party — complaints reference `party_id`)
- Own CSAT/NPS survey metrics or trending (stays in Fireproof's local `customer_satisfaction/` or external survey tooling like SurveyMonkey/Delighted — complaints are inbound incidents, not outbound survey aggregates)
- Own corrective-action workflow when a complaint triggers a CAPA (CAPA is Fireproof-only per user ruling; verticals handle their response in their own modules/overlays)
- Own customer communication (delegated to Notifications — complaint module emits events, Notifications sends the email/SMS)
- Own service ticket deflection or help-desk conversation threads (external help-desk tools own conversations; Customer-Complaints stores the canonical complaint record linked by opaque `external_ticket_ref`)
- Enforce regulatory response windows (aerospace has specific AS9100 customer-concern response windows; those live in Fireproof's overlay)

---

## 2. Domain Authority

Customer-Complaints is the **source of truth** for:

| Domain Entity | Authority |
|---------------|-----------|
| **Complaints** | Complaint lifecycle (intake → triaged → investigating → responded → closed), severity, source, category, title, description, customer_id, source entity (what it's about), assignee |
| **Complaint Activity Log** | Append-only record of investigation notes, status transitions, internal/external communications |
| **Complaint Resolution** | What action was taken, outcome, customer acceptance, closure timestamp |

Customer-Complaints is **NOT** authoritative for:
- The underlying customer party record (Party owns)
- The referenced source entity (e.g. the actual sales order or shipment — those modules own their records; complaint references by opaque UUID + type)
- Follow-up tasks/assignments (Workflow module owns task execution if used)

---

## 3. Data Ownership

All tables include `tenant_id`, shared-DB model.

| Table | Purpose | Key fields |
|-------|---------|-----------|
| **complaints** | Complaint record | `id`, `tenant_id`, `complaint_number`, `status` (canonical: intake/triaged/investigating/responded/closed/cancelled), `party_id` (ref → Party), `customer_contact_id` (nullable — specific contact who complained), `source` (canonical: phone/email/portal/survey/service_ticket/walk_in/letter/other), `source_ref` (opaque string — external ticket ID, email message-id, etc.), `severity` (canonical: low/medium/high/critical), `category` (open string; tenant-configurable), `title`, `description`, `source_entity_type` (nullable; e.g. sales_order/shipment/invoice/service_visit — what the complaint is about), `source_entity_id` (nullable UUID), `assigned_to`, `assigned_at`, `due_date`, `received_at`, `acknowledged_at`, `responded_at`, `closed_at`, `outcome` (canonical: resolved/unresolvable/customer_withdrew/duplicate; nullable until closed), `created_by`, `created_at`, `updated_at` |
| **complaint_activity_log** | Append-only history of status changes, investigation notes, communications | `id`, `tenant_id`, `complaint_id`, `activity_type` (canonical: status_change/note/customer_communication/internal_communication/attachment_added/assignment_change), `from_value` (nullable — for status changes), `to_value` (nullable), `content` (text), `visible_to_customer` (boolean — controls whether this entry is shown if Customer Portal surfaces the complaint), `recorded_by`, `recorded_at` |
| **complaint_resolutions** | Resolution record (one per complaint; closed-status prereq) | `id`, `tenant_id`, `complaint_id`, `action_taken` (text), `root_cause_summary` (text, optional), `customer_acceptance` (canonical: accepted/rejected/no_response/n_a), `customer_response_at` (nullable), `resolved_by`, `resolved_at` |
| **complaint_status_labels** | Per-tenant display labels over canonical status | Standard label-table shape |
| **complaint_severity_labels** | Per-tenant display labels over canonical severity | Standard label-table shape |
| **complaint_source_labels** | Per-tenant display labels over canonical source | Standard label-table shape |
| **complaint_category_codes** | Tenant-configured category taxonomy (open string + display label; unlike status/severity/source which are platform-canonical, category is tenant-defined) | `id`, `tenant_id`, `category_code`, `display_label`, `description`, `active`, `created_at`, `updated_at`, `updated_by` |

**Note on categories:** complaint taxonomy varies wildly by industry — aerospace has "customer concern letter / 8D / FAI deviation", waste has "missed pickup / damaged equipment / billing dispute", ranching has "livestock quality / service complaint." Platform does not impose a canonical category list. Each tenant registers its own codes in `complaint_category_codes`.

---

## 4. OpenAPI Surface

### 4.1 Complaint Endpoints
- `POST /api/customer-complaints/complaints` — Create complaint (enters `intake` status)
- `POST /api/customer-complaints/complaints/:id/triage` — intake → triaged (assignment + categorization confirmed)
- `POST /api/customer-complaints/complaints/:id/start-investigation` — triaged → investigating
- `POST /api/customer-complaints/complaints/:id/respond` — investigating → responded (customer has been communicated with)
- `POST /api/customer-complaints/complaints/:id/close` — responded → closed; requires resolution record
- `POST /api/customer-complaints/complaints/:id/cancel` — Cancel (e.g. duplicate, customer withdrew)
- `POST /api/customer-complaints/complaints/:id/assign` — Assign or reassign
- `GET /api/customer-complaints/complaints/:id` — Retrieve with activity log + resolution
- `GET /api/customer-complaints/complaints` — List (filters: status, severity, category, party_id, assigned_to, date ranges, source_entity_type)
- `PUT /api/customer-complaints/complaints/:id` — Update (while not closed)

### 4.2 Activity Log & Resolution
- `POST /api/customer-complaints/complaints/:id/notes` — Add investigation note (creates `activity_log` entry, `activity_type = note`)
- `POST /api/customer-complaints/complaints/:id/customer-communication` — Record customer communication (email/call summary)
- `POST /api/customer-complaints/complaints/:id/resolution` — Record resolution (prereq for close)
- `GET /api/customer-complaints/complaints/:id/activity-log` — Retrieve append-only log

### 4.3 Taxonomy & Label Endpoints
- `GET /api/customer-complaints/categories` — List tenant's category codes
- `POST /api/customer-complaints/categories` — Create category code
- `PUT /api/customer-complaints/categories/:code` — Update category
- `GET /api/customer-complaints/status-labels` — List tenant status display labels
- `PUT /api/customer-complaints/status-labels/:canonical` — Set status label
- `GET /api/customer-complaints/severity-labels` / `PUT .../:canonical` — Same for severity
- `GET /api/customer-complaints/source-labels` / `PUT .../:canonical` — Same for source

---

## 5. Events Produced & Consumed

Platform envelope applies. `source_module` = `"customer-complaints"`.

### 5.1 Events Produced

| Event name | Trigger | Key payload |
|------------|---------|-------------|
| `customer_complaints.complaint.received.v1` | Complaint created | `complaint_id`, `complaint_number`, `party_id`, `source`, `severity`, `category_code`, `source_entity_type`, `source_entity_id` |
| `customer_complaints.complaint.triaged.v1` | Transitioned to triaged | `complaint_id`, `assigned_to`, `category_code`, `severity` |
| `customer_complaints.complaint.status_changed.v1` | Any status transition | `complaint_id`, `from_status`, `to_status`, `transitioned_by` |
| `customer_complaints.complaint.assigned.v1` | Assignment or reassignment | `complaint_id`, `from_user`, `to_user`, `assigned_by` |
| `customer_complaints.complaint.customer_communicated.v1` | Customer communication logged | `complaint_id`, `communication_direction` (inbound/outbound), `recorded_by` |
| `customer_complaints.complaint.resolved.v1` | Resolution recorded | `complaint_id`, `customer_acceptance`, `resolved_by`, `resolved_at` |
| `customer_complaints.complaint.closed.v1` | Closed | `complaint_id`, `outcome`, `closed_at` |
| `customer_complaints.complaint.overdue.v1` | Daily sweep: `due_date < now()` and status not in (responded/closed/cancelled) | `complaint_id`, `assigned_to`, `due_date`, `days_overdue`, `severity` |

### 5.2 Events Consumed

| Event name | Source | Behavior |
|------------|--------|----------|
| `party.party.deactivated.v1` | Party | If open complaints exist for a deactivated party, log a warning activity entry; do not auto-close |
| `sales_orders.order.shipped.v1` | Sales-Orders | If a complaint references this order as its source_entity, log that shipment occurred (for context timeline) |
| `shipping_receiving.shipment.received.v1` | Shipping-Receiving | Same — context timeline |

Optional: modules that want to auto-create complaints (e.g. a missed-pickup event from a routing module) can POST directly to `customer-complaints/complaints` — no special subscription needed on platform side.

---

## 6. State Machines

### 6.1 Complaint Lifecycle
```
intake ──> triaged ──> investigating ──> responded ──> closed
   │          │              │                │
   └──────────┴──────────────┴────────────────┴──> cancelled
```
Terminal: `closed`, `cancelled`.

**Transition rules:**
- `intake → triaged`: requires category + severity + assignee.
- `triaged → investigating`: no prereq; investigation begins.
- `investigating → responded`: requires at least one `customer_communication` activity entry logged (confirms customer was communicated with).
- `responded → closed`: requires `complaint_resolution` record.
- `* → cancelled` from any non-terminal state (records reason in activity log).

### 6.2 Severity (canonical)
`low`, `medium`, `high`, `critical`

### 6.3 Source (canonical)
`phone`, `email`, `portal`, `survey`, `service_ticket`, `walk_in`, `letter`, `other`

### 6.4 Outcome (canonical — recorded on close)
`resolved`, `unresolvable`, `customer_withdrew`, `duplicate`

### 6.5 Customer Acceptance on Resolution (canonical)
`accepted`, `rejected`, `no_response`, `n_a`

---

## 7. Security & Tenant Isolation

- Shared DB, row-level isolation by `tenant_id`.
- Role gates: `customer_complaints:complaint:triage`, `customer_complaints:complaint:close`, `customer_complaints:complaint:cancel`, `customer_complaints:category:manage`, `customer_complaints:labels:edit`.
- Activity log entries have `visible_to_customer` flag to gate what Customer Portal shows if a vertical exposes complaints there.
- PII: complaint title/description may contain customer-identifying content. Standard soft-delete pattern for GDPR right-to-erasure (redact fields, do not physically delete for audit trail).

---

## 8. Required Invariants

1. **Close requires resolution.** Cannot close a complaint without at least one `complaint_resolution` record.
2. **Responded requires customer communication.** Cannot transition to `responded` without at least one `activity_type = customer_communication` entry.
3. **Activity log is append-only.** No updates or deletes on `complaint_activity_log`.
4. **Resolution is append-only per complaint.** Corrections create a new resolution row with later `resolved_at`; the complaint's effective resolution is the most recent row.
5. **Category must be active.** Cannot create a complaint with a `category_code` whose `active = false`. Historical complaints keep their category even if code is later deactivated.
6. **Severity and source are canonical.** Tenants rename via labels but cannot add/remove values.
7. **Due date drives overdue sweep.** `due_date` is auto-calculated on triage from a per-severity SLA table (platform default; tenant-configurable via a future extension). For MVP: default `due_date` = `received_at + 30 days` if not explicitly set.
8. **Tenant isolation cross-table.** All joins share `tenant_id`.
9. **Events carry canonical values.** Downstream modules match on canonical codes, not tenant display labels.

---

## 9. Cross-module integration notes

- **Party:** `party_id` references Party master. `customer_contact_id` (optional) references a specific Party contact person.
- **Sales-Orders / Shipping-Receiving / AR:** complaints often reference one of these as `source_entity`. Platform enforces nothing about the reference (it's a soft link across modules); UIs resolve it by calling the referenced module.
- **Notifications:** Customer-Complaints emits events; Notifications subscribes to drive customer-facing and internal emails/SMS on status transitions and overdue alerts.
- **Customer Portal (optional):** if the vertical exposes complaint status to customers via the portal, the portal reads complaints + filtered activity log entries (only `visible_to_customer = true`).
- **Integrations (optional):** external help-desk tools (Zendesk, Freshdesk) can create complaints via the REST API and populate `source_ref` with their ticket ID for round-trip linkage.

---

## 10. What stays in Fireproof (aerospace overlay, if needed)

Fireproof's aerospace-specific customer-concern workflow (AS9100 clause 8.2.1 specifics, formal customer concern letter format, AS9102 linkage to First Article deviations, response SLA tracked against AS9100-specific rules) runs as a Fireproof overlay service that:
- Subscribes to `customer_complaints.complaint.received.v1` and related events
- Stores AS9100-specific metadata per complaint in its own overlay table
- Enforces aerospace-specific rules that don't apply elsewhere (e.g. mandatory clause citation, specific response-time windows per severity)

Platform Customer-Complaints is complete and correct without any aerospace overlay. Waste/Ranching/Power verticals use it directly with their own category codes.

---

## 11. Open questions

- **Attachments.** Customers often attach photos/documents to complaints. Where do attachments live? Recommend: use platform `doc-mgmt` for attachment storage; complaint activity log entries reference doc IDs. No new storage in this module.
- **Complaint → CAPA linkage.** Fireproof's current feedback record has `capa_id`. Since CAPA is Fireproof-only, platform complaints don't have this FK. Fireproof's overlay stores the link on Fireproof's CAPA side. Other verticals can emit their own events to trigger their own response workflows.
- **SLA configuration.** Current design: fixed 30-day default, tenants explicitly set `due_date`. Alternative: tenant-configurable per-severity SLA table. Defer until real demand.
- **Duplicate detection.** If the same customer reports the same issue twice, should platform offer auto-detection? Recommend: no — keep simple, verticals build their own heuristics.
- **Multi-party complaints.** Complaint about an event involving multiple customers (e.g. a contamination affecting 10 customers). Current design: create 10 complaints with shared `source_entity_id`. Alternative: one complaint with a party list. Recommend separate records for auditability; correlate via `source_entity_id`.

---

## 12. Migration notes (from Fireproof)

- Fireproof's `customer_satisfaction/` module splits at migration:
  - **FeedbackRecord** (complaint record) → migrates to platform `customer_complaints.complaints`.
  - **SatisfactionPeriod / SatisfactionMetric / TrendPoint** (CSAT/NPS aggregate metrics) → **stays in Fireproof**. Not platform scope. Fireproof keeps this as-is.
- Fireproof's `FeedbackRecord.capa_id` drops on migration (CAPA stays in Fireproof; Fireproof overlay owns the complaint→CAPA link).
- Fireproof's `FeedbackRecord.sentiment` field drops; platform Customer-Complaints scopes specifically to complaints (negative by default), using `severity` instead.
- Fireproof's `period_id` drops (survey-period aggregation is a CSAT concern, not complaint concern).
- Sample data only — drop Fireproof's table, create fresh platform schema, Fireproof rewires to typed client (`platform_client_customer_complaints::*`).
- Initial category codes: verticals register their own on first use; platform seeds no defaults.
