# CRM-Pipeline Module — Scope, Boundaries, Contracts (draft v0.1)

**7D Solutions Platform**
**Status:** Draft Specification — bd-ixnbs migration
**Date:** 2026-04-16
**Proposed module name:** `crm-pipeline`
**Source of migration:** Fireproof ERP (`crm/` module, ~3,500 LOC — contacts portion is NOT migrated, reconciled against existing Party)
**Cross-vertical applicability:** All verticals — B2B sales motion applies to Fireproof (aerospace contracts), HuberPower (power-gen equipment), TrashTech (commercial waste contracts), RanchOrbit (livestock/breeding services)

---

## 1. Mission & Non-Goals

### Mission
The CRM-Pipeline module is the **authoritative system for the sales pipeline from lead intake through opportunity close**: capturing raw leads, qualifying them into opportunities, tracking stage progression with probability, logging sales activities against leads/opportunities/contacts, and handing off to Sales-Orders when an opportunity closes won.

### Non-Goals
CRM-Pipeline does **NOT**:
- Own contact or company records (delegated to Party — CRM references `party_id` and `party_contact_id`; no duplicate contact table)
- Own billing customer records or financial account (delegated to AR — opportunity closes won may trigger AR customer creation via event)
- Own quoting or RFQ generation (out of scope — verticals keep their own quoting; CRM opportunities reference `external_quote_ref` as opaque string)
- Own sales order creation (delegated to Sales-Orders — CRM emits `opportunity.closed_won.v1`, Sales-Orders or a vertical handler creates the SO)
- Own pricing logic (verticals set prices in their quoting tools)
- Own email/SMS delivery to leads (delegated to Notifications — CRM emits events, Notifications sends)
- Own marketing automation / nurture sequences (out of platform scope; use external tools)

---

## 2. Domain Authority

CRM-Pipeline is the **source of truth** for:

| Domain Entity | Authority |
|---------------|-----------|
| **Leads** | Pre-qualification records: source, company name (unconverted — not yet a Party), rough contact info, status (new → contacted → qualifying → qualified → converted / disqualified / dead), estimated value |
| **Opportunities** | Qualified deals: linked to a Party customer, stage progression, probability, expected/actual close dates, win/loss reason, owner, type, priority |
| **Opportunity Stage History** | Append-only log of stage transitions with time-in-stage, reason, notes |
| **Pipeline Stage Definitions** | Per-tenant configured pipeline stages — code, display label, order rank, is_terminal, is_win |
| **Activities** | Logged interactions against lead/opportunity/party_contact/party: activity_type, subject, description, date, duration, completion state, assignee |
| **Contact-Role Attributes** | CRM-specific attributes on top of Party contacts (sales role, decision-maker flag) — *not* the contact record itself (Party owns that) |

CRM-Pipeline is **NOT** authoritative for:
- Contact person details (Party owns — CRM references `party_contact_id`)
- Company details (Party owns — CRM references `party_id` on converted leads and opportunities)
- AR customer records (AR owns)
- Financial terms on a deal (quoting/SO owns those)

---

## 3. Data Ownership

All tables include `tenant_id`. Monetary values use integer `*_cents`.

| Table | Purpose | Key fields |
|-------|---------|-----------|
| **leads** | Pre-qualification records | `id`, `tenant_id`, `lead_number`, `source` (canonical: website/trade_show/referral/rfq/cold_call/existing_customer/other), `source_detail`, `company_name`, `contact_name`, `contact_email`, `contact_phone`, `contact_title`, `party_id` (nullable — set on conversion), `party_contact_id` (nullable — set on conversion), `status` (canonical: new/contacted/qualifying/qualified/converted/disqualified/dead), `disqualify_reason`, `estimated_value_cents`, `currency`, `converted_opportunity_id` (nullable), `converted_at` (nullable), `owner_id`, `notes`, `created_by`, `created_at`, `updated_at` |
| **opportunities** | Qualified deals | `id`, `tenant_id`, `opp_number`, `title`, `party_id` (ref → Party customer company), `primary_party_contact_id` (nullable — ref → Party contact), `lead_id` (nullable — back-ref to converted lead), `stage_code` (ref → pipeline_stages for this tenant), `probability_pct` (0-100), `estimated_value_cents`, `currency`, `expected_close_date`, `actual_close_date` (set when closed), `close_reason`, `competitor`, `opp_type` (canonical: new_business/repeat_order/contract_renewal/engineering_change/prototype), `priority` (canonical: low/medium/high/critical), `description`, `requirements`, `external_quote_ref` (opaque string for vertical's quoting system), `sales_order_id` (nullable — set when opportunity generated an SO), `owner_id`, `created_by`, `created_at`, `updated_at` |
| **opportunity_stage_history** | Stage transitions (append-only) | `id`, `tenant_id`, `opportunity_id`, `from_stage_code`, `to_stage_code`, `probability_pct_at_change`, `days_in_previous_stage`, `reason`, `notes`, `changed_by`, `changed_at` |
| **pipeline_stages** | Tenant-configured pipeline stage definitions | `id`, `tenant_id`, `stage_code`, `display_label`, `description`, `order_rank` (int — sort order in pipeline UI), `is_terminal` (boolean — stage ends the pipeline), `is_win` (boolean — terminal = won), `probability_default_pct` (nullable — default probability when opp enters this stage), `active`, `created_at`, `updated_at`, `updated_by` — unique on (`tenant_id`, `stage_code`) |
| **activities** | Interaction log | `id`, `tenant_id`, `activity_type_code` (ref → activity_types for this tenant), `subject`, `description`, `activity_date`, `duration_minutes`, `lead_id` (nullable), `opportunity_id` (nullable), `party_id` (nullable), `party_contact_id` (nullable), `due_date`, `is_completed`, `completed_at`, `assigned_to`, `created_by`, `created_at`, `updated_at` |
| **activity_types** | Tenant-configured activity type taxonomy | `id`, `tenant_id`, `activity_type_code`, `display_label`, `icon` (optional), `active`, `created_at`, `updated_at`, `updated_by` — unique on (`tenant_id`, `activity_type_code`) |
| **contact_role_attributes** | CRM-specific attributes on Party contacts | `id`, `tenant_id`, `party_contact_id`, `sales_role` (canonical: decision_maker/champion/influencer/user/blocker/unknown), `is_primary_buyer`, `is_economic_buyer`, `notes`, `updated_by`, `updated_at` — unique on (`tenant_id`, `party_contact_id`) |
| **lead_status_labels** | Per-tenant display labels over canonical lead status | Standard label-table shape |
| **lead_source_labels** | Per-tenant display labels over canonical lead source | Standard label-table shape |
| **opp_type_labels** | Per-tenant display labels over canonical opportunity type | Standard label-table shape |
| **opp_priority_labels** | Per-tenant display labels over canonical priority | Standard label-table shape |

**Important: pipeline stages are tenant-defined, NOT canonical.** Unlike status/severity/type fields elsewhere in platform, sales pipeline structure varies fundamentally by industry (short cycle vs long cycle, number of stages, stage meanings). Platform seeds default stages on tenant creation and lets the tenant modify via API. This is intentional.

**Default seed stages (tenant can modify):** `prospecting` → `discovery` → `proposal` → `negotiation` → `awaiting_commitment` → `closed_won` (terminal, is_win=true) | `closed_lost` (terminal, is_win=false).

---

## 4. OpenAPI Surface

### 4.1 Lead Endpoints
- `POST /api/crm-pipeline/leads` — Create lead (status `new`)
- `POST /api/crm-pipeline/leads/:id/contact` — new → contacted
- `POST /api/crm-pipeline/leads/:id/qualify` — contacted → qualifying
- `POST /api/crm-pipeline/leads/:id/mark-qualified` — qualifying → qualified
- `POST /api/crm-pipeline/leads/:id/convert` — qualified → converted; creates Party company (via Party API) + opportunity; body may include party_id if Party already exists
- `POST /api/crm-pipeline/leads/:id/disqualify` — Any status → disqualified (body: reason)
- `POST /api/crm-pipeline/leads/:id/mark-dead` — Any status → dead
- `GET /api/crm-pipeline/leads/:id` — Retrieve lead
- `GET /api/crm-pipeline/leads` — List with filters
- `PUT /api/crm-pipeline/leads/:id` — Update while not in terminal state

### 4.2 Opportunity Endpoints
- `POST /api/crm-pipeline/opportunities` — Create opportunity (starts at first non-terminal stage by order_rank)
- `POST /api/crm-pipeline/opportunities/:id/advance-stage` — Transition to a new stage (body: target stage_code, probability, reason, notes)
- `POST /api/crm-pipeline/opportunities/:id/close-won` — Move to closed_won terminal stage; may include sales_order_id
- `POST /api/crm-pipeline/opportunities/:id/close-lost` — Move to closed_lost terminal stage; body: close_reason
- `GET /api/crm-pipeline/opportunities/:id` — Retrieve with stage history and activities
- `GET /api/crm-pipeline/opportunities` — List with filters (owner, stage, party_id, close date ranges)
- `GET /api/crm-pipeline/opportunities/:id/stage-history` — List stage transitions
- `PUT /api/crm-pipeline/opportunities/:id` — Update non-stage fields
- `GET /api/crm-pipeline/pipeline/summary` — Aggregate: count & value by stage
- `GET /api/crm-pipeline/pipeline/summary?owner_id=X` — Filtered summary

### 4.3 Pipeline Stage Config
- `GET /api/crm-pipeline/stages` — List tenant's stages in order
- `POST /api/crm-pipeline/stages` — Add stage (body: code, display_label, order_rank, is_terminal, is_win, default_probability)
- `PUT /api/crm-pipeline/stages/:code` — Update
- `POST /api/crm-pipeline/stages/:code/deactivate` — Mark inactive (historical opportunities retain reference)
- `POST /api/crm-pipeline/stages/reorder` — Bulk reorder (body: array of code + new order_rank)

### 4.4 Activity Endpoints
- `POST /api/crm-pipeline/activities` — Log activity (against lead or opportunity or party or party_contact)
- `POST /api/crm-pipeline/activities/:id/complete` — Mark completed
- `GET /api/crm-pipeline/activities/:id` — Retrieve
- `GET /api/crm-pipeline/activities` — List with filters (assigned_to, due_before, completed, entity ref)
- `PUT /api/crm-pipeline/activities/:id` — Update
- `GET /api/crm-pipeline/activity-types` — List tenant's activity types
- `POST /api/crm-pipeline/activity-types` — Add activity type
- `PUT /api/crm-pipeline/activity-types/:code` — Update

### 4.5 Contact Role Attribute Endpoints
- `GET /api/crm-pipeline/contacts/:party_contact_id/attributes` — Get CRM attributes for a Party contact
- `PUT /api/crm-pipeline/contacts/:party_contact_id/attributes` — Set attributes

### 4.6 Label Endpoints
- Standard per-canonical-field: `/status-labels`, `/source-labels`, `/type-labels`, `/priority-labels` — all follow the same shape as in sales-orders / customer-complaints specs.

---

## 5. Events Produced & Consumed

Platform envelope. `source_module` = `"crm-pipeline"`.

### 5.1 Events Produced

| Event name | Trigger | Key payload |
|------------|---------|-------------|
| `crm_pipeline.lead.created.v1` | Lead created | `lead_id`, `lead_number`, `source`, `company_name`, `estimated_value_cents` |
| `crm_pipeline.lead.status_changed.v1` | Lead status transition | `lead_id`, `from_status`, `to_status`, `changed_by` |
| `crm_pipeline.lead.converted.v1` | Lead converted to opportunity | `lead_id`, `opportunity_id`, `party_id`, `party_contact_id` |
| `crm_pipeline.opportunity.created.v1` | Opportunity created | `opportunity_id`, `opp_number`, `party_id`, `stage_code`, `estimated_value_cents` |
| `crm_pipeline.opportunity.stage_advanced.v1` | Stage transition | `opportunity_id`, `from_stage_code`, `to_stage_code`, `probability_pct`, `days_in_previous_stage` |
| `crm_pipeline.opportunity.closed_won.v1` | Opportunity closed won | `opportunity_id`, `party_id`, `actual_close_date`, `estimated_value_cents`, `sales_order_id` (if set) |
| `crm_pipeline.opportunity.closed_lost.v1` | Opportunity closed lost | `opportunity_id`, `party_id`, `actual_close_date`, `close_reason`, `competitor` |
| `crm_pipeline.activity.logged.v1` | Activity recorded | `activity_id`, `activity_type_code`, `entity_type` (lead/opportunity/party/contact), `entity_id`, `assigned_to` |
| `crm_pipeline.activity.completed.v1` | Activity marked complete | `activity_id`, `completed_at`, `completed_by` |
| `crm_pipeline.activity.overdue.v1` | Daily sweep | `activity_id`, `assigned_to`, `due_date`, `days_overdue` |

### 5.2 Events Consumed

| Event name | Source | Behavior |
|------------|--------|----------|
| `party.party.deactivated.v1` | Party | Log warning on open opportunities; do not auto-close |
| `party.contact.deactivated.v1` | Party | Detach from opportunities (nullify primary_party_contact_id) |
| `sales_orders.order.booked.v1` | Sales-Orders | If the SO references an opportunity via soft linkage (downstream populates), log context; no state change |
| `ar.customer.created.v1` | AR | If lead conversion triggered AR customer creation, link up (future enhancement — not required for MVP) |

---

## 6. State Machines

### 6.1 Lead Lifecycle
```
new ──> contacted ──> qualifying ──> qualified ──> converted
  │         │             │              │
  └─────────┴─────────────┴──────────────┴──> disqualified
                                             or
                                             dead
```
Terminal: `converted`, `disqualified`, `dead`.

### 6.2 Opportunity Pipeline
Tenant-defined. Movement is governed by `pipeline_stages.order_rank` and `is_terminal`.
- Opportunities can move forward and backward between non-terminal stages freely (sales cycles are non-linear).
- Moving to an `is_terminal` stage requires explicit close-won or close-lost endpoint; close reason required on close-lost.
- Once in a terminal stage, opportunity cannot move to another stage (create a new opportunity if re-engaging).

### 6.3 Activity Lifecycle
```
created ──> completed
   │
   └──> (deleted — via separate endpoint, admin-only)
```
Activities can be updated until completed. Completed activities cannot revert (create a new one).

### 6.4 Contact Sales Role (canonical)
`decision_maker`, `champion`, `influencer`, `user`, `blocker`, `unknown`

---

## 7. Security & Tenant Isolation

- Shared DB, row-level isolation by `tenant_id`.
- Role gates: `crm_pipeline:lead:convert`, `crm_pipeline:opportunity:close`, `crm_pipeline:pipeline:config`, `crm_pipeline:activity_type:manage`.
- Activity logs may contain PII in descriptions; soft-delete pattern for GDPR.
- Ownership gating: opportunities/activities can be scoped to owner visibility (e.g. reps see only their own) — enforced at handler level via role + ownership check.

---

## 8. Required Invariants

1. **Lead conversion creates or links a Party.** A lead can only transition to `converted` if `party_id` is set (either pre-existing or created during conversion via Party API call from the handler).
2. **Opportunity stage must exist in pipeline_stages.** `opportunities.stage_code` must reference an active row in `pipeline_stages` for the tenant. Historical references to inactive stages remain valid for closed opportunities.
3. **Terminal stage requires explicit close endpoint.** Can't advance to a terminal stage via generic `advance-stage`; must use `close-won` or `close-lost`.
4. **Close-lost requires reason.** `close_reason` non-null when transitioning to a lost terminal stage.
5. **Pipeline must have exactly one initial stage.** The stage with lowest `order_rank` among active, non-terminal stages is the default entry stage for new opportunities.
6. **Stage history is append-only.** Corrections create a new history row with later `changed_at`.
7. **Activity must reference at least one entity.** At least one of `lead_id`/`opportunity_id`/`party_id`/`party_contact_id` is non-null.
8. **Tenant isolation cross-table.** All joins share `tenant_id`.
9. **Events carry canonical codes only.** Lead status, opp type, opp priority in events are canonical values. Stage codes are tenant-scoped; downstream consumers that need stage info receive the code + optionally look up display label via API.
10. **Contact role attributes reference active Party contacts.** `party_contact_id` must exist in Party. If the Party contact is deactivated, the attribute row remains but is flagged inactive via Party event subscription.

---

## 9. Cross-module integration notes

- **Party:** heavy integration. Lead conversion creates or references a Party company; opportunity contact is a Party contact. CRM never stores contact details of its own — always a `party_contact_id` reference.
- **Sales-Orders:** opportunity close-won optionally links a generated `sales_order_id`. The handoff flow (opp close-won → SO create) can be implemented as an event subscriber on Sales-Orders side or as a manual operator action; either works.
- **AR:** lead conversion or opportunity close-won may emit signals that an AR customer should be created. Current design: no automatic AR creation — vertical orchestrates this via their own event handler. Avoids tight coupling.
- **Notifications:** subscribes to lead/opportunity/activity events to fire emails (follow-up reminders, stage change alerts, overdue activity alerts).
- **Reporting:** queries opportunities and stage_history to build pipeline reports, weighted forecast, stage conversion rates.

---

## 10. Open questions

- **Contact table unification.** Current design: NO `crm_contacts` table — always reference Party. Fireproof's current CRM has a `crm_contacts` table which duplicates Party. Migration: drop Fireproof's `crm_contacts`, rewire to Party. Any CRM-specific contact attributes live in `contact_role_attributes`. Confirm this is the right approach or if there are CRM-specific contact fields not in Party that we need to preserve.
- **Lead-company-as-not-yet-Party.** Leads often come in before a Party exists (cold call to "some company I just heard about"). Current design: store raw `company_name`/contact info on the lead; Party record is created on conversion. If Party enforces uniqueness by name or email and a duplicate exists, conversion reuses the existing Party. OK to rely on handler logic.
- **Activity attachments.** Same as customer-complaints — use platform doc-mgmt for attachments; no module-local storage.
- **Opportunity split / spawn.** Some sales orgs split an opportunity when the deal gets re-scoped (original opp for half, new opp for half). Recommend: no split feature in v0.1 — close original, create new linked opp via `parent_opportunity_id` (cheap column to add now, surface later).
- **Team-based ownership.** Current design: single `owner_id` on lead/opportunity/activity. Sales teams often have co-owners or territory-based assignment. Recommend: add `co_owner_id` array or separate `opportunity_team` table later; single owner is fine for v0.1.
- **Forecasting weights.** Opportunity has `probability_pct` — summing `probability_pct * estimated_value_cents` across open opportunities gives weighted pipeline. Should platform provide a canonical weighted-pipeline endpoint in Reporting, or leave to verticals? Recommend platform Reporting owns it once at least one vertical asks.
- **Competitor tracking.** Current field: `competitor` free string. Teams often want structured competitor taxonomy. Defer until real demand.

---

## 11. Migration notes (from Fireproof)

- Fireproof's `crm/` module (~3,500 LOC) migrates:
  - **CrmLead → leads** — drop Fireproof's `customer_id` + `contact_id` FKs; use platform's `party_id` + `party_contact_id` set on conversion.
  - **CrmOpportunity → opportunities** — convert `BigDecimal` `estimated_value` to integer `estimated_value_cents`. Map Fireproof's hardcoded stage strings to seeded `pipeline_stages` rows for Fireproof's tenant on migration.
  - **OpportunityStageHistory → opportunity_stage_history** — same shape, `from_stage`/`to_stage` become `_code` fields.
  - **CrmActivity → activities** — Fireproof's hardcoded `ACTIVITY_TYPES` become seeded rows in `activity_types` table for Fireproof's tenant.
  - **CrmContact → DROPPED.** Fireproof's duplicate contact table is retired in favor of Party contacts. Fireproof's existing CRM contact data (where it has sales-role context) migrates to `contact_role_attributes` rows keyed by the matched Party contact.
  - **QuoteOrderLink → STAYS IN FIREPROOF.** Quoting is Fireproof-local; quote-to-order linkage logic stays with it.
- Fireproof's `contact_id` on lead and opportunity maps to `party_contact_id` after matching Fireproof CRM contacts to Party contacts (by email + name).
- Monetary `BigDecimal` fields convert to integer `*_cents`.
- Sample data — drop Fireproof tables, create platform schema, seed default pipeline_stages + default activity_types per tenant, Fireproof rewires to typed client (`platform_client_crm_pipeline::*`).
