# Tenant Control Plane UI — Product Vision

> **Phase 41**
> This document is the authoritative vision for the Tenant Control Plane staff-facing admin console.
> It survives agent context loss and is updated as decisions are made.

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | Platform Orchestrator | Created document. Navigation structure, design philosophy, language standards, full scope (A–H), design system, tab system, unsaved changes, column manager, modal system, Zustand stores, ESLint rules, technical constraints, open questions, decision log. |
| 1.1 | 2026-02-20 | Platform Orchestrator | Added Revision History, Decided By column to Decision Log, expanded rationale to include what was rejected. Moved to docs/frontend/TCP-UI-VISION.md. Adopted cross-app doc standard. |
| 1.2 | 2026-02-20 | Platform Orchestrator | Replaced all absolute Fireproof paths with repo-relative symlink paths (docs/reference/fireproof/). No content changes — path references only. |
| 1.3 | 2026-02-20 | Platform Orchestrator | Changed By and Decided By: replaced agent names with roles throughout (User, Orchestrator, TrashTech Orchestrator). Agent names are session-ephemeral. |
| 1.4 | 2026-02-20 | Platform Orchestrator | Added notification center to Global Shell scope (B). Added idle timeout as required for TCP. Added TCP-specific re-auth decision for Terminate action. Captured infrastructure decisions from Fireproof gap evaluation. |
| 1.5 | 2026-02-20 | Platform Orchestrator | Resolved all three remaining open questions (Q2, Q3, Q4). All open questions now resolved — bead creation can proceed. |
| 1.6 | 2026-02-20 | Platform Orchestrator | Added App Launcher section: cross-app navigation via shared auth cookie, per-app role model (dot-notation perms), Settings tab app cards, Access tab per-app role assignment UI. Updated F (IAM/Access) and C (Tenants) scope descriptions. |
| 1.7 | 2026-02-20 | Platform Orchestrator | Added Support Session section: Start Support Session from Access tab, time-limited impersonation JWT, customer-visible support session banner, early termination by customer, full audit trail. Updated F scope and actions list. |

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
- Start Support Session — on Access tab (see Support Sessions section)

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
- Top bar: staff identity, environment indicator, notification center bell (with count badge), logout
- Left navigation with all five sections, with badge counts on relevant nav items (e.g., Tenants showing past-due count)
- Idle timeout: 30-minute default, 5-minute warning modal before forced logout (required — TCP is a staff console with destructive actions)

### C. Tenants
- Tenant List: search, filter by status/plan/connection ID, pagination
- Tenant Detail: tabbed page (Overview, Billing, Access, Features, Settings, Activity)
- Lifecycle actions: Suspend, Activate, Terminate (with reason capture and confirmation)
- Connection mapping view and management
- App launcher: from Settings tab, a card for each app the tenant is subscribed to with a direct Launch link

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
- Per-app role assignment: each user shows their role in each subscribed app (admin / user / viewer / none). Staff can change a user's role per app independently.
- Seat leases: allocated vs active, release locked seat
- Active sessions: list, terminate session, policy violation flags
- Support Sessions: active support sessions for this tenant (who, which app, started when, expires when), with a "Start Support Session" action and ability to terminate an active session from TCP

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

Reference: `docs/reference/fireproof/src/styles/tokens.css` — port this directly, adapting values as needed.

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

Reference implementation: `docs/reference/fireproof/src/infrastructure/components/TabManager/` and `docs/reference/fireproof/src/infrastructure/state/tabStore.ts`

---

## App Launcher — Cross-App Navigation

### Concept

From the Settings tab of any Tenant Detail page, staff can see every app the tenant is subscribed to. Each app has a **Launch** button that opens the app in a new browser tab. The person launching arrives already authenticated, with their role in that specific app.

A user can be an admin in one app and a viewer in another. Roles are per-app, not global. The Access tab shows and manages this per-app role assignment for each user in the tenant.

### How authentication works at the launch link

The 7D Platform uses a shared auth domain. All apps (`trashtech.7d.io`, any future app, and TCP itself) share the same root cookie domain. The user's JWT (stored in an httpOnly cookie scoped to `.7d.io`) is valid at any app on that domain.

When a staff member clicks Launch:
1. Browser navigates to the app's URL (a new tab)
2. The app reads the existing JWT from the cookie — no re-login
3. The app checks the JWT's `perms` array for app-scoped permissions (e.g., `trashtech-pro.admin`)
4. Access is granted at whatever role the user holds in that app

No token exchange. No query-string tokens. The cookie does the work.

### Per-app roles in the JWT

The JWT `perms` field uses dot-notation. Per-app roles follow the same pattern:

| App | Role | Permission string in JWT |
|-----|------|--------------------------|
| TrashTech Pro | Admin | `trashtech-pro.admin` |
| TrashTech Pro | Dispatcher | `trashtech-pro.dispatcher` |
| TrashTech Pro | View-only | `trashtech-pro.viewer` |
| [Next App] | Admin | `[app-id].admin` |
| [Next App] | View-only | `[app-id].viewer` |

Each app defines its own permission strings and enforces them via `RequirePermissionsLayer`. TCP manages the assignment; apps enforce it.

### Settings tab — App Launcher UI

The Settings tab of Tenant Detail shows a **Subscribed Apps** section:

```
Subscribed Apps

[TrashTech Pro]          Status: Active    [ Launch → ]
[Another App]            Status: Active    [ Launch → ]
```

Each card shows: app name, subscription status, and the Launch button. If the current user has no role in that app, the Launch button still appears — they will arrive at the app and see its default no-role experience (typically a "you don't have access" state, handled by the app). TCP does not hide launch links based on the staff member's per-app role.

### Access tab — Per-App Role Management

The Access tab shows a user list. Selecting a user expands their role assignment across all subscribed apps:

```
Alice Chen
  TrashTech Pro    [ Admin ▼ ]
  Another App      [ Viewer ▼ ]

Bob Smith
  TrashTech Pro    [ Dispatcher ▼ ]
  Another App      [ — None — ]
```

Staff can change any role via the dropdown. Role changes take effect at the user's next login (JWT refresh). There is no mechanism to force an immediate session invalidation per-app — seat lease termination handles that if needed.

---

## Support Sessions

### Purpose

When a customer contacts support, a 7D staff member can open a time-limited session inside the customer's app to see exactly what they see and take actions on their behalf. The customer always knows when a support session is active — a persistent, non-dismissable banner appears on their screen the moment support logs in, and disappears the moment the session ends.

The customer can end the session at any time. Every support session is recorded in the audit log.

### Starting a session — from TCP

On the Access tab of any Tenant Detail page, the **Start Support Session** button opens a form:

- **App** — which subscribed app to access (dropdown)
- **Duration** — how long the session lasts (15 min / 30 min / 1 hour; default 30 min)
- **Reason** — required free-text field (shown in audit log and to the customer on the banner)

On confirm, the platform issues a support JWT and opens the app in a new browser tab. The support person sees the app exactly as the tenant's users do, authenticated under their own identity with `actor_type: "support"`.

The Access tab shows a **Support Sessions** panel listing any active sessions for this tenant: who started it, which app, when it expires. TCP staff can terminate an active session from this panel.

### The customer-facing banner

When a support session is active, a non-dismissable banner appears at the top of the app — on every screen, above everything else.

```
┌─────────────────────────────────────────────────────────────────────────┐
│  🔧  Support session active  —  Alex from 7D Solutions is logged in     │
│      Reason: "Help with route scheduling"  ·  Ends at 3:45 PM           │
│                                            [ End Session Now ]           │
└─────────────────────────────────────────────────────────────────────────┘
```

**Rules for the banner:**
- Non-dismissable — cannot be hidden or minimized. Only disappears when the session ends.
- Appears within 30 seconds of support logging in (driven by TanStack Query polling — the customer's app polls every 30 seconds for active support sessions on their account)
- Shows: support agent name, reason given, session expiry time
- "End Session Now" button terminates the session immediately — no confirmation required
- When the session ends (by expiry, by support leaving, or by customer terminating), the banner disappears within 30 seconds

### What the support person can do

The support person sees and can do everything the customer can do in that app, scoped to that tenant's data. They are not elevated above the tenant's own admin role — if the customer is a dispatcher, the support session operates at dispatcher level.

If support needs to take an admin action the customer cannot take, they do it from TCP (which is where staff-level platform actions live) — not from the support session.

### Audit log

Every support session generates audit events:
- `support_session.started` — who, which app, which tenant, duration, reason
- `support_session.ended` — how it ended (expiry / staff closed / customer terminated)
- `support_session.action` — any mutation the support person takes during the session is tagged `actor_type: "support"`, visible to the tenant in their own Activity log

The customer can see in their Activity log that support accessed their account and what actions were taken.

---

## Unsaved Changes Protection

Two layers of protection — neither is optional:

**Layer 1 — Browser close warning (`useBeforeUnload`):**
When a form is dirty, attempting to close the browser tab or window triggers a native browser warning. Disabled during E2E testing (via `VITE_DISABLE_UNLOAD_WARNING=true`) to prevent Playwright timeouts.

**Layer 2 — Unsaved Changes Panel (`UnsavedChangesPanel`):**
A collapsible panel shown on any form with pending changes. Shows a field-by-field diff of what changed: field name, "Was:" value, "Now:" value. Staff can see exactly what they'd lose before deciding to close.

**Tab close:** If a tab has `isDirty: true`, closing it opens a confirmation modal (not a native dialog) listing unsaved fields. Staff confirm before the tab closes.

Reference: `docs/reference/fireproof/src/infrastructure/hooks/useBeforeUnload.ts` and `docs/reference/fireproof/src/infrastructure/components/UnsavedChangesPanel.tsx`

---

## Column Management

Every data table in the application supports:
- **Show/hide columns** — toggle visibility per column
- **Drag-to-reorder** — drag column headers to rearrange
- **Persisted to backend API** — cross-device, not just localStorage
- **Tab-scoped** — each open tab maintains its own column configuration
- **Reset to default** — one button restores original column order and visibility

A dedicated "edit columns" mode toggles drag handles and visibility checkboxes on the table header. Changes apply and save when exiting edit mode.

Reference: `docs/reference/fireproof/src/infrastructure/hooks/useColumnManager.ts`

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

Reference: `docs/reference/fireproof/src/infrastructure/components/Modal.tsx`

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

Reference: `docs/reference/fireproof/eslint-local-rules/`

---

## Infrastructure Map

The Foundation bead must produce an `INFRASTRUCTURE_MAP.md` in `apps/tenant-control-plane-ui/src/infrastructure/` documenting every centralized system — what it is, how to use it, and what it replaces. This is the first document new agents read.

Reference: `docs/reference/fireproof/src/infrastructure/INFRASTRUCTURE_MAP.md`

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
| 2 | Does the landing page after login show the Tenant List, or a dashboard home with system health? | ✅ Resolved — Tenant List directly. See Decision Log. |
| 3 | Are there any screens that need charts or graphs (billing trends, seat usage over time)? | ✅ Resolved — No charts in Phase 41. See Decision Log. |
| 4 | Module versioning strategy — what does versioning mean for this platform? | ✅ Resolved — Not applicable to Phase 41. See Decision Log. |

---

## Decision Log

Decisions that are settled. Agents must not re-open these without an explicit user directive. Rationale includes what was considered and rejected.

| Date | Decision | Rationale (includes what was NOT chosen) | Decided By |
|------|----------|------------------------------------------|-----------|
| 2026-02-20 | Full product scope from day one — no MVP phasing | Phased approach requires tearing out and rebuilding structural elements later. Speed is not a constraint with a multi-agent worker pool. Rejected: MVP with 8 beads covering only tenant list + detail. | User |
| 2026-02-20 | Next.js (React) + TypeScript, not pure Rust (Leptos) | React ecosystem maturity, component tooling, developer availability, and shadcn/ui component library. Rejected: Leptos (immature ecosystem, limited component tooling, learning curve for agents). | User + Orchestrator |
| 2026-02-20 | Tenant Detail is a single tabbed page — not 8 separate sub-page navigations | Minimum clicks principle: one click from tenant list → one page with all context. Rejected: separate pages per section (billing page, users page, etc.) which would require navigating away to get full context. | User + Orchestrator |
| 2026-02-20 | Actions surface where the data is — not only in an Admin Tools section | Non-technical staff should not need to know which admin section handles what. Suspend button is on the tenant page. Rejected: centralized admin actions page. | User + Orchestrator |
| 2026-02-20 | Plain English labels throughout — no system terminology in UI | Tool must be usable by non-technical ops and support staff without documentation. Rejected: showing database field names, status codes, or internal IDs as display values. | User |
| 2026-02-20 | Color-rich UI with status badges (color + text) | Status must be scannable at a glance. Rejected: minimal/monochrome style (Linear/Vercel aesthetic) that requires reading text to determine status. | User |
| 2026-02-20 | Row / card view toggle on all list screens, preference persisted | Users have different workflows; some prefer dense row view, others prefer card view. Preference saved per user per table. Rejected: single fixed view mode. | User |
| 2026-02-20 | One-time charge feature removed from scope entirely | No current business use case identified. Can be added in a future bead if a need emerges. Rejected: keeping it in scope speculatively. | User |
| 2026-02-20 | Centralized design system — all components defined once in `components/ui/`, ESLint prevents ad-hoc variants | Consistency enforced by tooling, not discipline. Rejected: convention-based approach where developers are trusted to follow style guides. | User + Orchestrator |
| 2026-02-20 | CSS infrastructure via CSS custom properties in `globals.css` (same pattern as Fireproof `tokens.css`) | Single source of truth for all design values. Tailwind references CSS variables. Rejected: hardcoded Tailwind classes or inline hex values in components. | Platform Orchestrator |
| 2026-02-20 | Button double-click protection ON by default (1s cooldown) | Prevents duplicate submissions on financial/destructive actions. Rejected: opt-in protection (relies on developers remembering to enable it on important buttons). | Platform Orchestrator |
| 2026-02-20 | Tab system ported from Fireproof (preview/permanent, split view, tab-scoped state, isDirty persistence) | Staff work with multiple tenants simultaneously; tab state persists across refresh. Rejected: single-page-at-a-time navigation (forces context loss when switching). | Platform Orchestrator |
| 2026-02-20 | Column manager on all data tables — drag-reorder, show/hide, backend-persisted per user | Staff customize their view based on workflow; preferences sync across devices. Rejected: localStorage-only persistence (lost on device switch) or no customization. | Platform Orchestrator |
| 2026-02-20 | Unsaved changes: browser close warning + field-level diff panel (two layers) | Both are necessary — browser warning catches accidental navigation, diff panel gives staff information before they decide. Rejected: single-layer (warning only, no context about what would be lost). | Platform Orchestrator |
| 2026-02-20 | Modal: no backdrop-click close, two close behaviors (onClose vs onFullClose), portal rendering | Backdrop-click accidentally loses data. Two close behaviors distinguish cancel-and-stay from navigate-away. Rejected: backdrop-click close (high accidental loss rate on destructive actions). | Platform Orchestrator |
| 2026-02-20 | All persistent UI state in Zustand stores (tab-scoped) — no ad-hoc useState | State survives tab switches without data loss. ESLint rules make violations a build failure from day one. Rejected: component-local useState for persistent state (breaks on tab switch). | Platform Orchestrator |
| 2026-02-20 | Infrastructure Map document required in Foundation bead | First document new agents read before touching any UI code. Maps every centralized system. Rejected: relying on code comments and tribal knowledge. | Platform Orchestrator |
| 2026-02-20 | TCP UI implements idle timeout (30 min, 5 min warning) | Staff console with access to terminate tenants and modify billing. Session must expire on inactivity. Rejected: short JWT TTL only (no warning, loses form state). Rejected: no idle timeout (unattended sessions with admin access are a security risk). | Platform Orchestrator |
| 2026-02-20 | TCP UI implements re-authentication before Terminate Tenant action | Terminating a tenant is catastrophic and unrecoverable. Re-auth before this specific action ensures the session owner confirms the intent. Not a platform standard (risk profile is TCP-specific) — but required for TCP. Rejected: confirmation modal alone is sufficient (does not verify the operator is still the one at the keyboard). | Platform Orchestrator |
| 2026-02-20 | Notification center in top bar — TCP events: tenant past due, service health degraded, billing run complete | Staff may miss a 4-second toast. Notification center provides persistent history. These are the events significant enough to persist: billing and health events affect platform reliability. Rejected: toast only (missed events are lost). | Platform Orchestrator |
| 2026-02-20 | Landing page after login: Tenant List directly — no dashboard home | Design philosophy is minimum clicks to primary task. Staff use TCP primarily to work with tenants. A dashboard home screen adds a click for every session with no gain. System health (secondary use case) is one click away in the System section. Rejected: dashboard home with system health summary (adds a click to the most common task, system health is an edge-case action not a daily landing need). | Platform Orchestrator |
| 2026-02-20 | No charts or graphs in Phase 41 — all data as tables | Charts add charting library dependencies, visual design decisions, and responsive rendering complexity. All billing and usage data is accessible via tables on the Billing tab and invoice list. Rejected: adding charts for billing trends and seat usage (out of scope for Phase 41, can be added as a dedicated analytics bead if demand emerges). | Platform Orchestrator |
| 2026-02-20 | Module versioning: not applicable to Phase 41 | TCP UI is co-deployed with the platform in a monorepo. There is no independent versioning or API version prefix needed. The question was forward-looking; current scope has no versioning requirement. If the platform adopts versioned APIs in the future, the Consumer Guide will document it. Rejected: adding v1 URL prefixes speculatively (would require work throughout every BFF route with no current benefit). | Platform Orchestrator |
| 2026-02-20 | App launcher lives on Settings tab of Tenant Detail — one card per subscribed app with a Launch button | Minimum clicks: staff viewing a tenant's Settings can immediately jump to that app. Rejected: a dedicated top-level "Apps" section in TCP nav (too much distance from tenant context; launching an app is always tenant-specific). | User + Platform Orchestrator |
| 2026-02-20 | Cross-app authentication via shared auth cookie — no token exchange | All 7D apps share an auth domain. The httpOnly JWT cookie is valid across apps. Clicking Launch navigates to the app which reads the existing cookie. Zero friction, secure, no secrets in URLs. Rejected: query-string token hand-off (tokens in URLs appear in server logs and browser history). Rejected: per-app re-login (defeats the purpose of single sign-on). | Platform Orchestrator |
| 2026-02-20 | Per-app roles encoded as dot-notation permission strings in JWT (`{app-id}.{role}`) | The JWT `perms` field already uses dot-notation (`ar.mutate`). Extending it to per-app roles keeps one consistent model across all access control. Each app defines its own permission strings and enforces them via RequirePermissionsLayer. TCP manages assignment; apps enforce. Rejected: a separate per-app roles field in the JWT (would require JWT schema change; existing pattern already covers this). | Platform Orchestrator |
| 2026-02-20 | Role changes take effect at next JWT refresh — no forced per-app invalidation | Session invalidation is already handled by seat lease termination for cases requiring immediate effect. Adding per-app immediate invalidation would require platform-wide token revocation infrastructure. Rejected: forced immediate invalidation on role change (disproportionate complexity; seat termination already handles urgent cases). | Platform Orchestrator |
| 2026-02-20 | Support sessions implemented as time-limited impersonation JWT (`actor_type: "support"`) — not remote screen sharing | Built-in support access gives full audit trail, scoped access, and is professional-grade SaaS behavior. The customer always sees when support is active. Rejected: remote screen sharing only (no audit trail, support cannot see system-level data the customer doesn't have on screen, not scalable for a SaaS product). | User + Platform Orchestrator |
| 2026-02-20 | Customer sees a non-dismissable support session banner — always, for the full duration | Transparency is non-negotiable. The customer must always know when support is in their account. The banner cannot be hidden. Rejected: opt-in notification (customers might not see it). Rejected: notification-only with no persistent indicator (customer could forget support is active). | User + Platform Orchestrator |
| 2026-02-20 | Customer can terminate the support session at any time via the banner | Customer consent and control are platform values. A customer who did not initiate the session or wants it ended should be able to end it immediately without calling support. Rejected: only TCP staff can terminate sessions (removes customer agency). | User + Platform Orchestrator |
| 2026-02-20 | Support session banner polls every 30 seconds via TanStack Query — no WebSocket | Polling is sufficient for a session-start signal. The 30-second lag is acceptable for a support workflow (the customer and support agent are on a call together — a 30-second delay is not a UX problem). Rejected: WebSocket for real-time banner (adds infrastructure complexity for a feature that doesn't need sub-second latency). | Platform Orchestrator |
| 2026-02-20 | Support sessions operate at the tenant's own permission level — not elevated | Support navigates the app as the customer would, to understand and assist. If a staff-level action is needed, it is taken in TCP. Rejected: granting support elevated permissions inside the app (risks unintended changes and muddles the audit trail). | Platform Orchestrator |

---

> **Revision History** is at the top of this document (immediately after the header). See `docs/frontend/DOC-REVISION-STANDARDS.md` for the standards governing how this document is maintained.
