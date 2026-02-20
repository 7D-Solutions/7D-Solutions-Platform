# 7D Solutions Platform — Frontend Standards

> **Scope:** All business applications built on the 7D Solutions Platform.
> This document defines the shared foundation. App-specific decisions (color palette, navigation structure, audience) live in each app's own vision document.
>
> **Applies to:** Tenant Control Plane UI (Phase 41), TrashTech Pro, and all future platform apps.
> **Status:** ACTIVE — TrashTech Orchestrator reviewed. All 7 open items resolved. Document is product-agnostic.

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | Orchestrator | Created document. Guiding principles, tech stack, repository structure, CSS token system, Button/StatusBadge/Modal/DataTable/Form components, tab system, unsaved changes, state management, ESLint enforcement, language standards, navigation standards, testing standards, infrastructure map requirement, foundation bead checklist, app vision documents table. |
| 1.1 | 2026-02-20 | Orchestrator | Added Multi-Audience Apps section and Mobile Standards section (48×48px, 16px inputs, skeleton loaders, bottom nav, offline pattern). Incorporated TrashTech Orchestrator TrashTech audience input. |
| 1.2 | 2026-02-20 | Orchestrator | Added Toast vs Modal Threshold section and Real-Time Updates section. Added customer-facing language rules. |
| 1.3 | 2026-02-20 | Orchestrator | Genericized per user directive: removed all product-specific content. TrashTech status types, navigation model specifics, and audience names removed — belong in each app's vision doc. Updated StatusBadge to document audience prop and extension pattern. Clarified shared vs app-specific colors. |
| 1.4 | 2026-02-20 | Orchestrator | Added Revision History (this table), Decision Log, pointer to DOC-REVISION-STANDARDS.md. Adopted cross-app doc standard proposed by TrashTech Orchestrator. Moved to docs/frontend/ subfolder. Updated App Vision Documents paths. |
| 1.5 | 2026-02-20 | Orchestrator | Replaced all absolute paths with repo-relative symlink paths (docs/apps/trashtech/, docs/reference/fireproof/). No content changes — path references only. |
| 1.6 | 2026-02-20 | Orchestrator | Changed By and Decided By: replaced agent names with roles throughout. Agent names are session-ephemeral and must not appear in persistent documents. |

---

## Guiding Principles

### Minimum Clicks, Maximum Clarity
Reduce the number of steps to complete any task — but never at the expense of clarity or ease of use. Think of one-piece flow in manufacturing: it is the ideal, not a rigid rule. Three clicks that feel natural beat two clicks that are confusing.

### Built for Everyone
Apps must be usable by non-technical staff, customers, and field workers without documentation. If a label needs explanation, it is the wrong label.

### Context Before Action
Users should see full context before taking any action. Related information is accessible without navigating away from the current screen.

### Actions Live Where the Data Lives
If you are looking at data, the actions for that data are right there — not buried in a separate admin section.

### Centralization is Architecture
Consistency is enforced by structure, not discipline. Every shared UI element is defined once and imported everywhere. Ad-hoc implementations are prevented by tooling.

---

## Tech Stack

| Layer | Choice | Notes |
|-------|--------|-------|
| Framework | Next.js (App Router) + TypeScript | Server components, BFF routes, middleware auth |
| UI components | shadcn/ui + Tailwind | We own the source — components live in `components/ui/` |
| Styling | CSS custom properties + Tailwind | Tokens defined in `globals.css`, Tailwind references them |
| Data fetching | TanStack Query | Client-side, cache keys scoped per resource |
| Forms | React Hook Form + Zod | Validation at the boundary, not scattered |
| State | Zustand | Tab-scoped stores, persisted where needed |
| Testing | Playwright | E2E against real backend — no mocking, no MSW, no stubs |
| Auth | httpOnly cookies | JWTs never in localStorage or accessible to JavaScript |

> **Mobile:** The standard stack above applies to desktop/responsive apps (staff and customer portals). Mobile-first apps (e.g., TrashTech Driver) use the same stack but with distinct constraints — see **Multi-Audience Apps** section below.

---

## Repository Structure

Each app is self-contained within the platform monorepo:

```
apps/
  tenant-control-plane-ui/     # TCP UI — staff admin console
    app/                       # Next.js App Router pages + BFF routes
    components/
      ui/                      # CENTRALIZED — all shared components live here
    lib/
      api/                     # BFF client wrappers, types, query keys
      server/                  # Server-only utilities (bffFetch, auth guards)
    infrastructure/            # Zustand stores, hooks, services
      state/                   # All Zustand stores
      hooks/                   # Shared custom hooks
      services/                # userPreferencesService, etc.
      constants/               # modalPriority, pagination, toast config
    src/
      styles/                  # globals.css (tokens)
    INFRASTRUCTURE_MAP.md      # First document agents read — maps everything
  trashtech-pro/               # TrashTech — *(structure TBD with TrashTech Orchestrator)*
```

**Rule:** No app imports from another app's directory. Shared platform logic lives in the platform backend, not in frontend app code.

---

## CSS Infrastructure — Design Tokens

All design values are defined as CSS custom properties in `app/globals.css`. Tailwind is configured to reference these variables. Nothing is hardcoded in components.

**Token categories (ported from Fireproof `tokens.css`):**

### Color System
```css
:root {
  /* Primary */
  --color-primary: #2c72d5;
  --color-primary-light: #5691e3;
  --color-primary-dark: #1e5bb8;

  /* Semantic */
  --color-success: #28a745;
  --color-warning: #ffc107;
  --color-danger: #dc3545;
  --color-info: #17a2b8;

  /* Text */
  --color-text-primary: #212529;
  --color-text-secondary: #6c757d;
  --color-text-muted: #adb5bd;
  --color-text-inverse: #ffffff;

  /* Backgrounds */
  --color-bg-primary: #ffffff;
  --color-bg-secondary: #f8f9fa;
  --color-bg-tertiary: #e9ecef;

  /* Borders */
  --color-border-light: #dee2e6;
  --color-border-default: #ced4da;
}
```

> **Shared vs App-specific colors:**
> - **Semantic colors** (`--color-success`, `--color-warning`, `--color-danger`, `--color-info`, text, backgrounds, borders): **SHARED — never overridden per app.**
> - **Brand palette** (`--color-primary` and app-specific tokens): **App-specific.**
>   - Tenant Control Plane UI: `#2c72d5` (blue)
>   - TrashTech Pro: forest green + copper (defined in TrashTech vision doc)
>   - Future apps: define in their vision doc

### Typography, Spacing, Shadows, Z-Index
Full token reference: `docs/reference/fireproof/src/styles/tokens.css`
Port this directly into each app's `globals.css`.

### Component Size System
Shared sizing scale for buttons, badges, tags, inputs — ensures visual consistency across all interactive elements:

```css
/* Component sizes — shared by Button, Badge, Tag, Input */
--component-size-xs-padding-y: 6px;  --component-size-xs-min-height: 28px;
--component-size-sm-padding-y: 8px;  --component-size-sm-min-height: 32px;
--component-size-md-padding-y: 10px; --component-size-md-min-height: 38px;
--component-size-lg-padding-y: 12px; --component-size-lg-min-height: 44px;
--component-size-xl-padding-y: 16px; --component-size-xl-min-height: 52px;
--component-size-compact-padding-y: 5px; --component-size-compact-min-height: 26px;
```

### Layout Tokens
```css
--header-height: 77px;
--tab-bar-height: 48px;
--chrome-total-height: 155px; /* header + tab bar + margin */
```

---

## Centralized Component Library

**Rule enforced by ESLint:** No raw `<button>` elements, no `window.confirm()`, no inline status rendering, no ad-hoc modal implementations anywhere in the codebase. Every interactive element is imported from `components/ui/`.

### Button

**Variants** (semantic — tied to meaning, not decoration):

| Variant | Color | When to use |
|---------|-------|-------------|
| `primary` | Brand blue | Main action (Save, Confirm, Assign) |
| `secondary` | Neutral | Supporting actions (Edit, View, Export) |
| `success` | Green | Positive completion actions |
| `danger` | Red | Destructive actions (Terminate, Delete, Revoke) |
| `warning` | Amber | Caution actions (Suspend, Force-release) |
| `info` | Teal | Informational actions |
| `ghost` | Transparent | Tertiary / low-emphasis actions |
| `outline` | Border only | Alternative secondary style |

**Sizes:**

| Size | Min height | When to use |
|------|-----------|-------------|
| `compact` | 26px | Dense toolbars, tight table rows |
| `xs` | 28px | In-table row actions |
| `sm` | 32px | Secondary actions, sidebars |
| `md` | 38px | Default — most page-level actions |
| `lg` | 44px | Primary CTA |
| `xl` | 52px | Prominent confirmation dialogs |

**Built-in behaviors:**
- **Double-click protection**: ON by default (1000ms cooldown). Prevents duplicate submissions on financial and destructive actions.
- **Loading state**: Spinner replaces content while `loading={true}`. Button auto-disables.
- **Icon support**: Optional leading icon via `icon` prop.
- **Active state**: `active` prop for toggleable nav buttons.

Usage: `<Button variant="danger" size="sm">Terminate</Button>`

### StatusBadge

All status rendering goes through `<StatusBadge status="active" />`. Color, label, and icon are determined by the component from a central config map — never by the calling page.

**Variants:** `default` | `compact` | `large`

**Audience prop (optional):** `<StatusBadge status="completed" audience="driver" />` — selects the right label for the user's context. Values: `admin` (default) | `driver` | `customer`. When omitted, defaults to `admin`.

**Platform status types (shared — never removed or recolored per app):**

| Status key | Color | Label |
|-----------|-------|-------|
| `active` | Green | Active |
| `suspended` | Amber | Suspended |
| `terminated` | Red | Terminated |
| `pending` | Blue | Setting up |
| `past_due` | Red | Past Due |
| `degraded` | Amber | Degraded |
| `unknown` | Gray | Unknown |
| `available` | Green | Available |
| `unavailable` | Red | Unavailable |

**App-specific status types:** Each app extends the status config map with its own types defined in that app's vision document. App-specific types are added to a separate `appStatusConfigs` map that is merged into the platform config at app initialization. Platform types are never removed or recolored by app extensions.

**Audience-aware labels:** The same status can have different display labels for different user audiences (e.g., staff vs. customer vs. field worker). Use the optional `audience` prop to select the appropriate label: `<StatusBadge status="completed" audience="customer" />`. Each app defines its own audience values. Default is `admin`.

> **Rule:** Never render status inline. Never hardcode a color based on a status string. All status rendering goes through `<StatusBadge>`.

### Modal

**Rules:**
- Never use `window.confirm()` or `window.alert()` — use `Modal` component
- No backdrop-click close — intentional, prevents accidental data loss
- Escape key closes (unless `preventClosing` is set)
- Always renders via React portal to `document.body`
- Z-index managed automatically — nested modals stack correctly

**Two close behaviors:**
- `onClose` — dismiss/cancel, stay in current context
- `onFullClose` — X button, navigate back to parent page

**Sizes (from CSS tokens):**

| Size | Width | Use |
|------|-------|-----|
| `sm` | 480px | Simple confirmations, alerts |
| `md` | 600px | Standard forms |
| `lg` | 800px | Complex forms, detail views |
| `xl` | 1000px | Multi-section workflows |

**Composition pattern:**
```tsx
<Modal isOpen={isOpen} onClose={onClose} onFullClose={onFullClose} size="md" title="Suspend Tenant">
  <Modal.Body>...</Modal.Body>
  <Modal.Actions>
    <Button variant="ghost" onClick={onClose}>Cancel</Button>
    <Button variant="warning" loading={isPending}>Suspend</Button>
  </Modal.Actions>
</Modal>
```

### DataTable / ViewToggle

All list screens support two display modes toggled by the user:

- **Row view** — compact table, more records visible, best for scanning
- **Card view** — richer per-record display, best for detail at a glance

The toggle is a shared `<ViewToggle />` component. User preference is persisted per table per user (backend API, cross-device).

Column management is built into every DataTable:
- Show/hide columns
- Drag-to-reorder columns
- Persisted to backend API per user (cross-device sync)
- Tab-scoped — each open tab maintains its own column configuration
- Reset to default available

### Form Components

All form inputs come from `components/ui/`. Never use raw HTML `<input>`, `<select>`, `<textarea>`.

| Component | Replaces |
|-----------|---------|
| `FormInput` | `<input type="text">` |
| `FormSelect` | `<select>` |
| `FormTextarea` | `<textarea>` |
| `FormCheckbox` | `<input type="checkbox">` in forms |
| `Checkbox` | `<input type="checkbox">` in tables/grids |
| `NumericFormInput` | `<input type="number">` |
| `SearchableSelect` | `<select>` with search |
| `DateRangePicker` | Date range inputs |

---

## Tab System

Apps use a browser-tab-like interface. Users can have multiple records open simultaneously without losing context.

**Tab behaviors:**
- **Preview tabs** (shown in italics): browsing, viewing — replaced by the next navigation
- **Permanent tabs**: creating, actively editing — stay until explicitly closed
- Tabs **persist across browser refresh** (Zustand + localStorage)
- Tab state is **scoped**: each tab maintains its own form data, filters, search, column config, and open modals
- **Split view**: two tabs side by side with draggable divider (20%-80%)
- Tabs are drag-to-reorderable
- Right-click context: Close / Close Others / Close All

**isDirty protection:**
Tabs with unsaved changes show a dirty indicator. Closing a dirty tab opens a confirmation modal listing the unsaved fields — not a native browser dialog.

> **Mobile:** Tab system is **desktop-only**. Mobile views (TrashTech Driver app) use standard stack-based page navigation. No tabs, no split view, no column manager on mobile.

---

## Unsaved Changes Protection

Two layers — both required on any screen with a form:

**Layer 1 — Browser close warning:**
`useBeforeUnload(isDirty)` — triggers native browser warning when user tries to close the tab or window with unsaved changes. Disabled during E2E testing via `VITE_DISABLE_UNLOAD_WARNING=true`.

**Layer 2 — Unsaved Changes Panel:**
Collapsible panel shown on the page when form is dirty. Displays a field-by-field diff: field name, "Was:" value, "Now:" value. Staff see exactly what they would lose.

---

## State Management

All UI state that should survive a tab switch lives in a Zustand store. No ad-hoc `useState` for anything persistent.

**Stores (all tab-scoped):**

| Store | Purpose | Persistence |
|-------|---------|-------------|
| `tabStore` | Tab list, active tab, split view | localStorage |
| `modalStore` | Which modal is open and with what data | In-memory |
| `useFormStore` | Form field values | localStorage |
| `useFilterStore` | Filter state + active filter detection | localStorage |
| `useSearchStore` | Search term + recent search history | localStorage |
| `useUploadStore` | File upload metadata and progress | localStorage |
| `useSelectionStore` | Checkbox / multi-select state | localStorage |
| `useViewStore` | Active tab index, current step, collapsed sections | localStorage |

**User preferences (backend-persisted, cross-device):**
Column configurations and view preferences are saved to the backend API via `userPreferencesService` — not just localStorage.

---

## ESLint Enforcement

Custom ESLint rules are active from day one. Violations fail the build. No exceptions without a documented justification.

| Rule | What it prevents |
|------|----------------|
| `no-raw-button` | Raw `<button>` elements — use `Button` component |
| `no-local-modal-state` | `useState` for modal open/close — use `modalStore` |
| `no-local-form-state` | `useState` for form fields — use `useFormStore` |
| `no-local-filter-state` | `useState` for filters — use `useFilterStore` |
| `no-local-search-state` | `useState` for search — use `useSearchStore` |
| `no-local-upload-state` | `useState` for file uploads — use `useUploadStore` |
| `no-local-selection-state` | `useState` for selections — use `useSelectionStore` |
| `no-local-view-state` | `useState` for tab/step/collapse — use `useViewStore` |

Reference implementation: `docs/reference/fireproof/eslint-local-rules/`

---

## Language Standards

Users should never see internal system terminology. Labels must be plain English.

**Rules (apply to all apps):**
- Never show database column names in the UI (no `tenant_id`, `app_id`, `ar_customer_id`)
- Never show system codes as display values (no `DELINQUENT`, `IN_CALIBRATION`)
- Boolean fields displayed as "Yes / No" — not `true / false`
- Dates formatted with locale — never raw ISO strings
- Currency formatted with `Intl.NumberFormat` — never raw numbers
- Error messages state what happened and what the user should do next

**Customer-facing apps (additional rules):**
- Errors must include a contact method: "We couldn't load this — try again or call 555-0100" not "Error 503"
- Dates in conversational format: "Tuesday, March 4th at 8:14am" — never any ISO format
- No confirmation dialogs for non-destructive actions the user initiated deliberately (one tap to pay, one tap to confirm)
- Maximum 3 data columns visible at once. Prefer cards over tables for customer views.

---

## Navigation Standards

**Flat over nested.** Navigation menus have one level. No dropdowns inside dropdowns.

**Minimum clicks target:**
- 1 click to reach a section
- 1 more click to reach a specific record
- Actions available on the record — no extra navigation

**Tabs on detail pages** instead of separate pages for sub-sections. All information about a record is accessible via tabs on its detail page.

---

## Testing Standards

- **Playwright E2E only** — no unit tests for UI behavior
- **Real backend** — no mocking, no MSW, no stubs, no fake API responses
- **loginAs fixture** — each app defines a fixture for each user role (e.g., `loginAsStaff()`, `loginAsDriver()`, `loginAsCustomer()`)
- Every screen has a Playwright spec covering: login → navigation → at least one real data read (or empty state)
- E2E disabled during CI if backend is not available — never fake the backend to make tests pass

---

## Infrastructure Map (Required in Every App)

Every app must maintain an `INFRASTRUCTURE_MAP.md` in its `infrastructure/` directory. This document:
- Lists every centralized system with its file path and purpose
- Provides usage examples
- Is the first document an agent reads before touching any UI code
- Is updated whenever new infrastructure is added

Reference: `docs/reference/fireproof/src/infrastructure/INFRASTRUCTURE_MAP.md`

---

## Foundation Bead Requirements

Every app's Foundation bead must deliver all of the following before any feature bead ships:

- [ ] `app/globals.css` — full CSS token system
- [ ] `tailwind.config.ts` — tokens wired in
- [ ] `components/ui/Button.tsx` — all variants, sizes, double-click protection, loading state
- [ ] `components/ui/StatusBadge.tsx` — platform status types + app-specific extensions
- [ ] `components/ui/Modal.tsx` — all behaviors, composition pattern
- [ ] `components/ui/ViewToggle.tsx` — row/card toggle
- [ ] `components/ui/DataTable.tsx` — with column manager
- [ ] `components/ui/index.ts` — single import point
- [ ] `infrastructure/state/` — all Zustand stores
- [ ] `infrastructure/hooks/useBeforeUnload.ts` — browser close protection
- [ ] `infrastructure/hooks/useColumnManager.ts` — column persistence
- [ ] `infrastructure/services/userPreferencesService.ts` — backend preference persistence
- [ ] `eslint-local-rules/` — all enforcement rules active
- [ ] `INFRASTRUCTURE_MAP.md` — complete
- [ ] Playwright auth fixture for each user role
- [ ] CI configuration

---

## Multi-Audience Apps

Some apps serve multiple distinct audiences with radically different needs (e.g., staff, customers, field workers). Each audience is treated as a separate app within the platform monorepo, sharing the component library but having distinct navigation, layout, and interaction patterns.

**Audience tiers:**

| Audience tier | Typical needs | Platform features that apply |
|---------------|--------------|------------------------------|
| Staff / admin (desktop) | Dense data, multi-record work, full actions | Full standard: tabs, column manager, row/card toggle, modals |
| Customer-facing (responsive) | Simplified flow, max 3-5 screens, plain language | Platform components; no tab system, no column manager |
| Field worker (mobile-first) | One-handed, glanceable, offline tolerance | See Mobile Standards below |

**Rule:** Never try to serve desktop and mobile-first in the same Next.js route/layout. Separate routes, separate layouts — shared component library underneath.

Each app's vision document defines which audience tiers it serves and what the navigation model is for each.

---

## Mobile Standards (Driver-class Apps)

Applies to mobile-first apps (TrashTech Driver, any future field worker apps). These are not "responsive desktop apps" — they are distinct products with a shared component library underneath.

### Core Constraints (all enforced, not aspirational)

- **Touch targets:** 48×48px CSS minimum on every interactive element. This is enforced, not aspirational.
- **Form input font size:** 16px minimum. Below 16px triggers iOS auto-zoom on tap, which breaks the driver's flow.
- **Body text:** 14px minimum. Line height minimum 1.5.
- **Primary navigation:** Bottom navigation bar only. Maximum 5 items. No left sidebar on any mobile view.
- **Primary orientation:** Portrait. Must also function in landscape without layout breakage.
- **No horizontal scrolling** on any mobile screen.
- **Skeleton loaders only** (no spinners) on route and stop screens — drivers expect to see content immediately.
- **One-handed use:** Primary actions reachable without stretching.
- **No tab system:** Standard stack navigation only. No browser-tab-like interface.
- **No column manager:** Mobile views use fixed, optimized layouts.
- **No split view.**
- **No multi-step modal flows:** If a confirmation is needed, show a full-screen confirmation page — not a modal layered over the route screen. Modals on mobile are disorienting.

### Mobile Navigation Model

**Pattern:** Flat drill-down. List view is the home screen. Tap a record → detail view. Back button returns to the list. No tabs. Each app's vision document defines the specific sections and labels.

### Offline State Management

Apps that need offline tolerance (field worker apps) must define in their vision document what specifically works offline vs. requires connectivity. The implementation pattern is:

- **IndexedDB** for local mutation queue
- **TanStack Query** with `networkMode: 'offlineFirst'` for read operations
- **Sync-on-reconnect** for queued mutations — all queued operations must be idempotent
- **Service worker** for critical data caching on app open (so the user can view data immediately)

The app's vision document specifies which operations must work offline for that product. Do not add offline support for operations that are not listed there.

### Foundation Bead Additions (Mobile Apps)
In addition to the standard Foundation bead checklist, mobile apps add:
- [ ] Offline detection hook (`useNetworkStatus`)
- [ ] IndexedDB mutation queue service
- [ ] Sync-on-reconnect handler
- [ ] Service worker for route data caching
- [ ] Bottom navigation component
- [ ] Touch-target audit (automated test verifying 48px CSS minimum)

---

## Toast vs Modal Threshold

Rules for agents — not guidelines.

**Use a toast when:**
- An action succeeded and requires no further input (save, submit, status change)
- A background process finished (export ready, sync complete)
- Toast duration: **4 seconds**. Auto-dismiss. No action required from user.
- Maximum **one toast visible at a time.** Queue if a second fires before the first dismisses.

**Use a modal when:**
- The action is **destructive or irreversible** (delete, terminate, cancel)
- The action requires **a reason or confirmation input** from the user
- The action **affects billing** (any invoice or payment action)
- Confirmation language must name what will happen specifically: "Cancel this order?" — not "Are you sure?"

**Field worker apps:** No multi-step modal flows. If a confirmation is needed, show a full-screen confirmation page — not a modal layered over the work screen.

---

## Real-Time Updates

**MVP standard: TanStack Query polling.**
- Use `refetchInterval` in TanStack Query for live-updating data.
- The polling interval is defined as a named constant in `lib/constants.ts` — never hardcoded in a component.
- Never use raw `setInterval` for data polling. Use TanStack Query's `refetchInterval` exclusively.

**Phase 2 trigger for WebSocket/SSE:** If polling creates visible lag at scale (many concurrent users/entities), upgrade to WebSocket or SSE. The threshold and trigger condition are defined in the app's vision document.

---

## Open Items

- [ ] **Customer-facing language rules:** What additional plain-English rules apply specifically to customer-facing screens? Each app that serves customers should define these in its vision document. (Guideline: errors must tell users what to do next and include a contact method.)
- [ ] **Standard app scaffold:** Should the platform provide a starter template repo that new apps fork from? Defer until second app is ready to begin.
- [ ] **Shared package vs. per-app copy:** Should the component library live in a shared package (`packages/ui/`) that apps install as a dependency, or does each app own its own copy? Defer until duplication becomes a maintenance problem.

---

## App Vision Documents

Each app maintains its own vision document with app-specific decisions:

| App | Vision Document |
|-----|----------------|
| Tenant Control Plane UI | `docs/frontend/TCP-UI-VISION.md` (this repo) |
| TrashTech Pro | `docs/apps/trashtech/VISION.md` |

---

## Decision Log

Standards decisions that are settled. Agents must not re-open these without an explicit user directive. Rationale includes what was considered and rejected.

| Date | Decision | Rationale (includes what was NOT chosen) | Decided By |
|------|----------|------------------------------------------|-----------|
| 2026-02-20 | Next.js (App Router) + TypeScript as the standard frontend framework | Mature ecosystem, strong TypeScript support, server components, BFF routes built in. Rejected: Vite + React (no BFF), Remix (less adoption), SvelteKit (agent unfamiliarity). | User + Orchestrator |
| 2026-02-20 | CSS custom properties in `globals.css` as the token system — not Tailwind config values directly | CSS variables cascade, can be overridden per component or theme without rebuilding Tailwind. Rejected: hardcoding Tailwind classes in components (no single source of truth). | Orchestrator |
| 2026-02-20 | Semantic colors shared across apps — brand palette app-specific | Semantic meaning (success=green, danger=red) must be consistent platform-wide. Brand color is per-product identity. Rejected: fully unified visual theme across all apps (ignores product identity). | User + TrashTech Orchestrator |
| 2026-02-20 | Tab system is desktop-only — mobile views use stack navigation | Tabs are a desktop interaction pattern. Mobile users navigate with back button + flat drill-down. Rejected: responsive tab system (too complex, poor mobile UX). | TrashTech Orchestrator (confirmed) |
| 2026-02-20 | ESLint rules enforced from day one — violations fail build, no override comments | Consistency by tooling not discipline. If rules can be bypassed in emergencies, they will be bypassed routinely. Rejected: lint warnings (ignored), convention-based standards (drift over time). | Orchestrator |
| 2026-02-20 | TanStack Query `refetchInterval` for real-time data — WebSocket/SSE deferred to Phase 2 | Polling is sufficient for MVP, simpler to implement and debug. WebSocket adds operational complexity. Rejected: WebSocket from day one (over-engineering for initial scale). | TrashTech Orchestrator |
| 2026-02-20 | Playwright E2E against real backend only — no mocking, no MSW, no stubs | Tests that mock the backend don't catch backend contract changes. Real integration is the only valid proof. Rejected: MSW mocking (fast but doesn't catch real failures). | User |
| 2026-02-20 | Document is product-agnostic — all app-specific content belongs in each app's vision doc | Platform doc must be usable by any future app team without mental search-and-replace. Rejected: including TrashTech-specific examples inline (confuses future app teams). | User |

---

> **Revision History** is at the top of this document (immediately after the header). See `docs/frontend/DOC-REVISION-STANDARDS.md` for the standards governing how this document is maintained.
