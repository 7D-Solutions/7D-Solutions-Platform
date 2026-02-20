# Tenant Control Plane UI — Product Vision

> **Phase 41**
> This document is the authoritative vision for the Tenant Control Plane staff-facing admin console.
> It survives agent context loss and is updated as decisions are made.

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | BrightHill | Created document. Navigation structure, design philosophy, language standards, full scope (A–H), design system, tab system, unsaved changes, column manager, modal system, Zustand stores, ESLint rules, technical constraints, open questions, decision log. |
| 1.1 | 2026-02-20 | BrightHill | Added Revision History, Decided By column to Decision Log, expanded rationale to include what was rejected. Moved to docs/frontend/TCP-UI-VISION.md. Adopted TopazElk cross-app doc standard. |

---

## What Are We Building?

A staff-facing admin console. The people using this are 7D Solutions employees — not tenants, not customers. Staff use it to manage the entire multi-tenant platform from one place.

**Core jobs this tool does:**
- Find any tenant and see everything about them at a glance
- Take administrative actions on tenants (suspend, activate, terminate, change plan)
- Manage what features a tenant has access to
- Monitor billing, invoices, and payment status per tenant
- Manage users, roles, and sessions within a tenant
- Run administrative operations (trigger billing, reconcile mappings)
- Watch system health and platform-wide audit activity

---

## Who Uses This?

7D Solutions staff — which includes engineers but also operations and support personnel who may not be technical. The UI must be usable by someone seeing it for the first time without needing documentation or training. This is not a power-user-only tool.

---

## Design Philosophy

### Minimum Clicks — As a Principle, Not a Rule

Think of one-piece flow in manufacturing: it is the ideal to aim for, not a rigid constraint. The goal is to minimize the number of clicks to complete any task — but never at the expense of clarity or ease of use.

**The target:**
- 1 click to reach a section (left navigation)
- 1 more click to reach a specific tenant or record
- Actions available right there — no extra navigation required

When more clicks are genuinely necessary, the path must be obvious and natural. Three clicks that feel intuitive beat two clicks that are confusing.

### Built for Everyone

Language, labels, and layout must make sense to a non-technical staff member. If a label requires explanation, it is the wrong label.

**Rules:**
- Plain English throughout — no system terminology, no database column names, no internal codes
- Status is immediately obvious — color + text, never just a code
- Actions are clearly labeled with what they do, not what they call internally
- Errors and confirmations explain what happened in plain language
- Nothing requires reading documentation to understand

### Context Before Action

Staff must be able to see full context before taking any action. You should never need to navigate away from a tenant to understand what is happening with them. All relevant information is visible on their page.

### Actions Live Where the Data Lives

If you are looking at a user in the Access tab, "Deactivate" is right there — not buried in a separate Users page. If you are looking at billing, "Create one-time charge" is a button on the screen. Administrative tools surface in context, not only in a separate Admin section.

---

## Navigation Structure

### Top Level — Left Navigation (always visible, flat, no nesting)

| Section | What lives here |
|---------|----------------|
| **Tenants** | The primary view. Search, filter, and manage any tenant. |
| **Plans & Pricing** | Platform plan catalog — what plans exist, what they include, what they cost. |
| **Bundles & Features** | Feature bundles and individual feature entitlements. |
| **Audit & Activity** | Platform-wide activity log with filters. |
| **System** | Platform health status and administrative tools. |

Five sections. Flat. No nested navigation menus.

### The Tenant Detail Page — The Workhorse

The most-used screen in the application. One click from the Tenant List lands you here. Everything about a tenant is accessible from this page via tabs. You do not navigate away to see their billing, their users, or their features.

**Tabs:**

| Tab | What is here |
|-----|-------------|
| **Overview** | Status, current plan, health snapshot, key dates, quick stats |
| **Billing** | Current charges, invoice list, payment status, dunning state, one-time charge |
| **Access** | Users, roles, seat leases, active sessions |
| **Features** | Active features (plan + bundle + overrides), feature override actions |
| **Settings** | Connection mapping, plan assignment, setup status |
| **Activity** | Audit log filtered to this tenant |

**Actions visible on Tenant Detail (not buried):**
- Suspend / Activate / Terminate — prominent buttons, shown/hidden by current status
- Change Plan — on Billing tab and Overview
- Create one-time charge — on Billing tab
- Release a locked seat — on Access tab
- End a session — on Access tab
- Grant / revoke a feature override — on Features tab

---

## Language Standards

Staff should never see internal system terminology. The table below is the translation guide.

| Internal / System Term | Staff-Facing Label |
|------------------------|-------------------|
| `tenant_id → app_id registry` | Connection mapping |
| `dunning state: DELINQUENT` | Past due |
| `force-release lease` | Release locked seat |
| `platform_admin` | Staff admin |
| `cp_plans` | Platform plans |
| `provisioning_state` | Setup status |
| `entitlement override` | Feature override |
| `effective entitlements` | Active features |
| `concurrent sessions` | Active sessions |
| `merchant_context` | Never shown to users |
| `app_id` | Connection ID (shown in technical context only) |

---

## Scope — All Screens

The full product. No phased MVP that requires tearing out and rebuilding. Agents work through it sequentially but the architecture — navigation, routing, types, BFF layer — is designed for the complete product on the first pass.

### A. Foundation
- Next.js App Router scaffold with all navigation slots and route placeholders from day one
- Staff authentication (httpOnly cookie, `platform_admin` enforcement)
- Shared type system and BFF framework
- Playwright auth fixture (`loginAsStaff`)

### B. Global Shell
- Top bar: staff identity, environment indicator, logout
- Left navigation with all five sections

### C. Tenants
- Tenant List: search, filter by status/plan/connection ID, pagination
- Tenant Detail: tabbed page (Overview, Billing, Access, Features, Settings, Activity)
- Lifecycle actions: Suspend, Activate, Terminate (with reason capture and confirmation)
- Connection mapping view and management

### D. Plans & Pricing
- Plans catalog: all platform plans with pricing model, included seats, metered dimensions
- Plan detail: pricing rules, associated bundles and features
- Tenant plan assignment: change plan with effective date
- Billing overview per tenant: current charges (base + seats + metered)
- Invoice list and invoice detail (line items)
- Payment and past-due status

### E. Bundles & Features
- Bundles catalog and bundle detail (composition)
- Features catalog
- Tenant active features view (plan + bundle + overrides combined, labeled by source)
- Per-tenant feature override: grant/revoke with required justification

### F. IAM / Access
- Tenant users: list, deactivate
- Roles and permissions: assign roles, manage tenant-scoped access
- Seat leases: allocated vs active, release locked seat
- Active sessions: list, terminate session, policy violation flags

### G. Audit & Activity
- Platform-wide audit log: filter by tenant, actor, action type, date range

### H. System
- Service health: readiness status for all backend services
- Administrative tools: run billing, reconcile connection mapping

---

## Design System — Centralized Components

**Non-negotiable rule: no ad-hoc styling of interactive elements anywhere in the application.**
Every button, badge, status indicator, and color is defined once in `components/ui/` and pulled from there. Feature screens never create their own variants.

### Buttons

Defined by two properties: **variant** (what it does semantically) + **size** (where it lives in the layout).

**Variants:**
| Variant | Color | When to use |
|---------|-------|-------------|
| `primary` | Brand color (blue) | Main action on a page (Save, Confirm, Assign) |
| `secondary` | Neutral | Supporting actions (Edit, View, Export) |
| `danger` | Red | Destructive actions (Terminate, Deactivate, Revoke) |
| `warning` | Amber | Caution actions (Suspend, Force-release) |
| `ghost` | Transparent | Tertiary / low-emphasis actions |

**Sizes:**
| Size | When to use |
|------|-------------|
| `sm` | In-table row actions, compact toolbars |
| `md` | Default — most page-level actions |
| `lg` | Primary call-to-action, confirmation dialogs |

Usage: `<Button variant="danger" size="sm">Terminate</Button>` — that is all. No inline styles, no one-off Tailwind classes.

### Status Badges

All status rendering goes through a single `<StatusBadge status="active" />` component. Color and label are determined by the component — never by the calling page.

| Status | Color | Label |
|--------|-------|-------|
| `active` | Green | Active |
| `suspended` | Amber | Suspended |
| `terminated` | Red | Terminated |
| `pending` | Blue | Setting up |
| `past_due` | Red | Past Due |
| `degraded` | Amber | Degraded |
| `unknown` | Gray | Unknown |

### Color Palette

Colors are semantic — tied to meaning, not decoration. Defined in Tailwind config as named tokens, never as raw hex values in components.

| Token | Meaning |
|-------|---------|
| `success` | Healthy, active, complete |
| `warning` | Caution, attention needed |
| `danger` | Error, destructive, past due |
| `info` | Neutral status, in-progress |
| `muted` | Inactive, unknown, disabled |

### CSS Infrastructure (Design Tokens)

All design values live as CSS custom properties in `app/globals.css` — the same pattern used in the previous Fireproof build. Tailwind is configured to reference these variables. Nothing is hardcoded in components.

Token categories defined here:
- **Colors** — semantic (success, warning, danger, info, muted) + brand palette + grays
- **Typography** — font families, sizes, weights, line heights
- **Spacing** — base scale (0.25rem increments)
- **Component sizes** — shared padding/height/font-size scale used by Button, Badge, Tag, Input, etc.
- **Layout** — header height, sidebar width, modal sizes, container breakpoints
- **Shadows** — elevation scale
- **Border radius** — named scale
- **Z-index** — named layers (dropdown, modal, tooltip, notification)
- **Transitions** — named duration + easing values

Reference: `/Users/james/Projects/Fireproof/frontend/src/styles/tokens.css` — port this directly, adapting values as needed.

### What the Foundation Bead Must Deliver

The design system is established in the Foundation bead. No feature bead ships until this exists:
- `app/globals.css` — full CSS token system (ported from Fireproof tokens.css)
- `tailwind.config.ts` — configured to reference CSS variables as named tokens
- `components/ui/Button.tsx` — all variants and sizes, **double-click protection on by default** (1s cooldown), loading state with spinner, icon support
- `components/ui/StatusBadge.tsx` — status config map, icon support, compact/default/large variants
- `components/ui/ViewToggle.tsx` — row/card toggle (shared by all list screens, preference persisted)
- `components/ui/index.ts` — single import point for all UI components
- Rule enforced in code comments: never use raw `<button>` or inline status rendering — always import from `components/ui/`

---

## Tab System

The application uses a browser-tab-like interface — staff can have multiple tenants open simultaneously, compare plans side by side, or be mid-edit on one tenant while switching to another without losing work.

**Behavior (ported from Fireproof tabStore):**
- Tabs open automatically when navigating to a new route
- **Preview tabs** (italics): browsing a list or viewing a record — can be replaced by the next click
- **Permanent tabs**: creating something, actively editing — stay open until explicitly closed
- Closing the active tab switches to the previous tab gracefully
- Tab state **persists across browser refresh** (Zustand + localStorage)
- Tabs cannot be closed while `isDirty` (unsaved changes) without confirmation
- **Split view**: two tabs open side by side with a draggable divider (20%-80% range)
- Tabs are reorderable via drag-and-drop
- Right-click context menu: Close, Close Others, Close All

**Tab scoping:** All state (form data, filters, search, column config, modals) is scoped to the active tab ID. Switching tabs switches context completely without losing data in either tab.

Reference implementation: `/Users/james/Projects/Fireproof/frontend/src/infrastructure/components/TabManager/` and `/infrastructure/state/tabStore.ts`

---

## Unsaved Changes Protection

Two layers of protection — neither is optional:

**Layer 1 — Browser close warning (`useBeforeUnload`):**
When a form is dirty, attempting to close the browser tab or window triggers a native browser warning. Disabled during E2E testing (via `VITE_DISABLE_UNLOAD_WARNING=true`) to prevent Playwright timeouts.

**Layer 2 — Unsaved Changes Panel (`UnsavedChangesPanel`):**
A collapsible panel shown on any form with pending changes. Shows a field-by-field diff of what changed: field name, "Was:" value, "Now:" value. Staff can see exactly what they'd lose before deciding to close.

**Tab close:** If a tab has `isDirty: true`, closing it opens a confirmation modal (not a native dialog) listing unsaved fields. Staff confirm before the tab closes.

Reference: `/Users/james/Projects/Fireproof/frontend/src/infrastructure/hooks/useBeforeUnload.ts` and `/infrastructure/components/UnsavedChangesPanel.tsx`

---

## Column Management

Every data table in the application supports:
- **Show/hide columns** — toggle visibility per column
- **Drag-to-reorder** — drag column headers to rearrange
- **Persisted to backend API** — cross-device, not just localStorage
- **Tab-scoped** — each open tab maintains its own column configuration
- **Reset to default** — one button restores original column order and visibility

A dedicated "edit columns" mode toggles drag handles and visibility checkboxes on the table header. Changes apply and save when exiting edit mode.

Reference: `/Users/james/Projects/Fireproof/frontend/src/infrastructure/hooks/useColumnManager.ts`

---

## Modal System

All modals use the centralized `Modal` component. No `window.confirm()`, no `window.alert()`, no ad-hoc modal implementations.

**Key behaviors:**
- **No backdrop-click close** — intentional, prevents accidental data loss
- **Escape key closes** (unless `preventClosing` prop is set)
- **Two close behaviors**: `onClose` (dismiss/cancel, stay in context) vs `onFullClose` (X button — navigate back to main page)
- **Proper stacking** — z-index managed automatically, nested modals stack correctly
- **Portal rendering** — always renders to `document.body`, never trapped in parent layout
- **Size from tokens** — sm / md / lg / xl, never hardcoded pixel widths
- **Composition pattern**: `Modal.Header`, `Modal.Body`, `Modal.Actions`, `Modal.Tabs` — consistent structure everywhere

Reference: `/Users/james/Projects/Fireproof/frontend/src/infrastructure/components/Modal.tsx`

---

## State Management Infrastructure

All UI state is centralized in Zustand stores, tab-scoped, and persisted. No ad-hoc `useState` for anything that should survive a tab switch.

| Store | Purpose | Persisted |
|-------|---------|-----------|
| `tabStore` | Tab list, active tab, split view | localStorage |
| `modalStore` | Which modal is open, with what data | No (in-memory) |
| `useFormStore` | Form field values across tab switches | localStorage |
| `useFilterStore` | Filter state with active filter detection | localStorage |
| `useSearchStore` | Search term + recent searches history | localStorage |
| `useUploadStore` | File upload metadata and progress | localStorage |
| `useSelectionStore` | Checkbox/multi-select state | localStorage |
| `useViewStore` | Active tab index, step, collapsed sections | localStorage |

**User preferences (backend-persisted, cross-device):**
Column configurations are saved to the backend API via `userPreferencesService` — not just localStorage. This means a staff member's column layout follows them across devices.

---

## ESLint Enforcement

Custom ESLint rules enforce the infrastructure — no agent or developer can accidentally use ad-hoc state:

| Rule | What it blocks |
|------|---------------|
| `no-local-modal-state` | Raw `useState` for modal open/close |
| `no-local-form-state` | Raw `useState` for form fields |
| `no-local-filter-state` | Raw `useState` for filters |
| `no-local-search-state` | Raw `useState` for search |
| `no-local-upload-state` | Raw `useState` for file upload state |
| `no-local-selection-state` | Raw `useState` for selections |
| `no-local-view-state` | Raw `useState` for tab/step/collapse |

These rules are active from day one. Violations fail the build.

Reference: `/Users/james/Projects/Fireproof/frontend/eslint-local-rules/`

---

## Infrastructure Map

The Foundation bead must produce an `INFRASTRUCTURE_MAP.md` in `apps/tenant-control-plane-ui/src/infrastructure/` documenting every centralized system — what it is, how to use it, and what it replaces. This is the first document new agents read.

Reference: `/Users/james/Projects/Fireproof/frontend/src/infrastructure/INFRASTRUCTURE_MAP.md`

---

## Technical Constraints (for agents, not for UI discussion)

- **Stack**: Next.js (App Router) + TypeScript, shadcn/ui + Tailwind, TanStack Query, React Hook Form + Zod, Playwright
- **Auth**: Staff JWT stored in httpOnly cookie only; `platform_admin` role enforced at middleware and every BFF route; browser never calls Rust APIs directly
- **BFF**: All data calls go through Next.js API routes which proxy to Rust backend services
- **Location in repo**: `apps/tenant-control-plane-ui/` — self-contained with its own `package.json`
- **Tests**: Playwright E2E against real backend — no mocking, no MSW, no stubs
- **Rust backend**: NOT modified by any Phase 41 beads
- **CI**: Additive — npm build + Playwright tests run alongside existing cargo tests

---

## Open Questions

These are decisions still to be made. Do not create beads until these are resolved.

| # | Question | Status |
|---|----------|--------|
| 1 | Visual style | ✅ Resolved — see Decision Log |
| 2 | Does the landing page after login show the Tenant List, or a dashboard home with system health? | Open |
| 3 | Are there any screens that need charts or graphs (billing trends, seat usage over time)? | Open |
| 4 | Module versioning strategy — what does versioning mean for this platform? | Open |

---

## Decision Log

Decisions that are settled. Agents must not re-open these without an explicit user directive. Rationale includes what was considered and rejected.

| Date | Decision | Rationale (includes what was NOT chosen) | Decided By |
|------|----------|------------------------------------------|-----------|
| 2026-02-20 | Full product scope from day one — no MVP phasing | Phased approach requires tearing out and rebuilding structural elements later. Speed is not a constraint with a multi-agent worker pool. Rejected: MVP with 8 beads covering only tenant list + detail. | User |
| 2026-02-20 | Next.js (React) + TypeScript, not pure Rust (Leptos) | React ecosystem maturity, component tooling, developer availability, and shadcn/ui component library. Rejected: Leptos (immature ecosystem, limited component tooling, learning curve for agents). | User + BrightHill |
| 2026-02-20 | Tenant Detail is a single tabbed page — not 8 separate sub-page navigations | Minimum clicks principle: one click from tenant list → one page with all context. Rejected: separate pages per section (billing page, users page, etc.) which would require navigating away to get full context. | User + BrightHill |
| 2026-02-20 | Actions surface where the data is — not only in an Admin Tools section | Non-technical staff should not need to know which admin section handles what. Suspend button is on the tenant page. Rejected: centralized admin actions page. | User + BrightHill |
| 2026-02-20 | Plain English labels throughout — no system terminology in UI | Tool must be usable by non-technical ops and support staff without documentation. Rejected: showing database field names, status codes, or internal IDs as display values. | User |
| 2026-02-20 | Color-rich UI with status badges (color + text) | Status must be scannable at a glance. Rejected: minimal/monochrome style (Linear/Vercel aesthetic) that requires reading text to determine status. | User |
| 2026-02-20 | Row / card view toggle on all list screens, preference persisted | Users have different workflows; some prefer dense row view, others prefer card view. Preference saved per user per table. Rejected: single fixed view mode. | User |
| 2026-02-20 | One-time charge feature removed from scope entirely | No current business use case identified. Can be added in a future bead if a need emerges. Rejected: keeping it in scope speculatively. | User |
| 2026-02-20 | Centralized design system — all components defined once in `components/ui/`, ESLint prevents ad-hoc variants | Consistency enforced by tooling, not discipline. Rejected: convention-based approach where developers are trusted to follow style guides. | User + BrightHill |
| 2026-02-20 | CSS infrastructure via CSS custom properties in `globals.css` (same pattern as Fireproof `tokens.css`) | Single source of truth for all design values. Tailwind references CSS variables. Rejected: hardcoded Tailwind classes or inline hex values in components. | BrightHill |
| 2026-02-20 | Button double-click protection ON by default (1s cooldown) | Prevents duplicate submissions on financial/destructive actions. Rejected: opt-in protection (relies on developers remembering to enable it on important buttons). | BrightHill |
| 2026-02-20 | Tab system ported from Fireproof (preview/permanent, split view, tab-scoped state, isDirty persistence) | Staff work with multiple tenants simultaneously; tab state persists across refresh. Rejected: single-page-at-a-time navigation (forces context loss when switching). | BrightHill |
| 2026-02-20 | Column manager on all data tables — drag-reorder, show/hide, backend-persisted per user | Staff customize their view based on workflow; preferences sync across devices. Rejected: localStorage-only persistence (lost on device switch) or no customization. | BrightHill |
| 2026-02-20 | Unsaved changes: browser close warning + field-level diff panel (two layers) | Both are necessary — browser warning catches accidental navigation, diff panel gives staff information before they decide. Rejected: single-layer (warning only, no context about what would be lost). | BrightHill |
| 2026-02-20 | Modal: no backdrop-click close, two close behaviors (onClose vs onFullClose), portal rendering | Backdrop-click accidentally loses data. Two close behaviors distinguish cancel-and-stay from navigate-away. Rejected: backdrop-click close (high accidental loss rate on destructive actions). | BrightHill |
| 2026-02-20 | All persistent UI state in Zustand stores (tab-scoped) — no ad-hoc useState | State survives tab switches without data loss. ESLint rules make violations a build failure from day one. Rejected: component-local useState for persistent state (breaks on tab switch). | BrightHill |
| 2026-02-20 | Infrastructure Map document required in Foundation bead | First document new agents read before touching any UI code. Maps every centralized system. Rejected: relying on code comments and tribal knowledge. | BrightHill |

---

> **Revision History** is at the top of this document (immediately after the header). See `docs/frontend/DOC-REVISION-STANDARDS.md` for the standards governing how this document is maintained.
