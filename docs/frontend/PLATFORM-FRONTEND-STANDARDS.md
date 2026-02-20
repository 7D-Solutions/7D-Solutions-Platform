# 7D Solutions Platform — Frontend Standards

> **Scope:** All business applications built on the 7D Solutions Platform.
> This document defines the shared foundation. App-specific decisions (color palette, navigation structure, audience) live in each app's own vision document.
>
> **Applies to:** Tenant Control Plane UI (Phase 41), TrashTech Pro, and all future platform apps.
> **Status:** ACTIVE — TrashTech Orchestrator reviewed. All 7 open items resolved. Document is product-agnostic.

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | Platform Orchestrator | Created document. Guiding principles, tech stack, repository structure, CSS token system, Button/StatusBadge/Modal/DataTable/Form components, tab system, unsaved changes, state management, ESLint enforcement, language standards, navigation standards, testing standards, infrastructure map requirement, foundation bead checklist, app vision documents table. |
| 1.1 | 2026-02-20 | Platform Orchestrator | Added Multi-Audience Apps section and Mobile Standards section (48×48px, 16px inputs, skeleton loaders, bottom nav, offline pattern). Incorporated TrashTech Orchestrator TrashTech audience input. |
| 1.2 | 2026-02-20 | Platform Orchestrator | Added Toast vs Modal Threshold section and Real-Time Updates section. Added customer-facing language rules. |
| 1.3 | 2026-02-20 | Platform Orchestrator | Genericized per user directive: removed all product-specific content. TrashTech status types, navigation model specifics, and audience names removed — belong in each app's vision doc. Updated StatusBadge to document audience prop and extension pattern. Clarified shared vs app-specific colors. |
| 1.4 | 2026-02-20 | Platform Orchestrator | Added Revision History (this table), Decision Log, pointer to DOC-REVISION-STANDARDS.md. Adopted cross-app doc standard proposed by TrashTech Orchestrator. Moved to docs/frontend/ subfolder. Updated App Vision Documents paths. |
| 1.5 | 2026-02-20 | Platform Orchestrator | Replaced all absolute paths with repo-relative symlink paths (docs/apps/trashtech/, docs/reference/fireproof/). No content changes — path references only. |
| 1.6 | 2026-02-20 | Platform Orchestrator | Changed By and Decided By: replaced agent names with roles throughout. Agent names are session-ephemeral and must not appear in persistent documents. |
| 1.7 | 2026-02-20 | Platform Orchestrator | Formalized governance model: platform is single source of truth, app teams follow not fork. Converted Open Items to Deferred Decisions table. Genericized App Vision Documents to a registry with registration date. Added governance decisions to Decision Log. |
| 1.8 | 2026-02-20 | Platform Orchestrator | Added five new sections: Notification System (browser notifications prohibited — platform notifications only, toast + notification center), Standard Hooks (useMutationPattern, useQueryInvalidation, usePagination, useSearchDebounce, useLoadingState), Formatter Standards (date + currency rules), Idle Timeout (standardized pattern for staff-facing apps), Nav Badge Counts (standardized pattern for any sidebar nav). Added ESLint prohibition on browser Notification API. Updated Foundation Bead checklist. Added 9 new Decision Log entries from joint evaluation with TrashTech Orchestrator. |

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

## Standard Hooks

Every TanStack Query app needs these hooks. Not standardizing them means each app builds slightly different versions that diverge over time. Each app implements these locally — no shared package, no versioning overhead. The signature and behavior contract is what is standardized.

Reference implementations: `docs/reference/fireproof/src/infrastructure/hooks/`

| Hook | Purpose | Signature contract |
|------|---------|-------------------|
| `useMutationPattern` | Standardized API mutations | Accepts `mutationFn`, returns `{ mutate, isPending, error }`. Error is always surfaced to the caller — never swallowed. Loading state auto-managed. |
| `useQueryInvalidation` | Cache invalidation after mutations | Accepts query keys to invalidate. Called inside `onSuccess` of mutations. Never invalidate all queries blindly — be explicit. |
| `usePagination` | Centralized pagination | Returns `{ page, pageSize, totalCount, totalPages, goToPage, nextPage, prevPage }`. Page is 1-indexed. Default pageSize defined as a named constant in `lib/constants.ts`. |
| `useSearchDebounce` | Debounced search input | Accepts `value` and optional `delay` (default 300ms). Returns debounced value. Delay is configurable per usage — some searches need 150ms, some 500ms. |
| `useLoadingState` | Coordinated loading across multiple operations | Returns `{ isLoading, setLoading, withLoading }`. Prevents multiple spinners competing. |

**Rule:** Never use raw `setTimeout` for debounce. Never manually track loading state with `useState`. Never invalidate query cache with wildcard keys.

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
| `no-browser-notifications` | `new Notification(...)` or `Notification.requestPermission()` — use platform notification system |

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

## Formatter Standards

Every app implements a local `infrastructure/utils/formatters.ts` following these rules exactly. No app invents its own date or currency format.

### Date Formatting

| Context | Format | Example |
|---------|--------|---------|
| Within 7 days | Relative | "2 hours ago", "Yesterday at 3pm", "3 days ago" |
| Beyond 7 days | Short date | "Feb 20, 2026" |
| Audit / activity events | Short date + time, always | "Feb 20, 2026 at 10:37am" |
| Long dates | Never | — |

**Rules:**
- Never render a raw ISO string in the UI (`2026-02-20T10:37:00Z` is never user-facing)
- Always include time for audit log entries and activity feeds — timestamp without time is useless for debugging
- Relative times use the smallest meaningful unit: "just now" (< 60s), "2 minutes ago", "1 hour ago", "Yesterday at 3pm", then fall through to short date

### Currency Formatting

- Always use `Intl.NumberFormat` — never format currency manually
- Always include currency symbol
- Currency code comes from the data — never hardcoded to USD
- Format: `new Intl.NumberFormat('en-US', { style: 'currency', currency: record.currency })`
- For multi-currency contexts, show both symbol and code: "$1,234.56 USD"

### Numeric Formatting

- Percentages: one decimal place (`12.3%`) — never raw decimal (`0.123`)
- Large numbers: comma-separated (`1,234,567`) — use `Intl.NumberFormat`
- Decimal precision: matches the domain (financial = 2 decimal places, percentage = 1)

---

## Navigation Standards

**Flat over nested.** Navigation menus have one level. No dropdowns inside dropdowns.

**Minimum clicks target:**
- 1 click to reach a section
- 1 more click to reach a specific record
- Actions available on the record — no extra navigation

**Tabs on detail pages** instead of separate pages for sub-sections. All information about a record is accessible via tabs on its detail page.

**Nav badge counts:** Nav items may display a numeric count badge (e.g., "3 past due", "4 unassigned"). The hook signature and badge placement are standardized — badge appears top-right of the nav item, numeric, no color override. Data source is app-specific. Each app implements a `useBadgeCounts` hook that returns a `Record<navKey, number>` and passes counts to the nav component. Zero counts hide the badge.

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
- [ ] `infrastructure/hooks/useMutationPattern.ts` — standardized API mutations
- [ ] `infrastructure/hooks/useQueryInvalidation.ts` — cache invalidation
- [ ] `infrastructure/hooks/usePagination.ts` — centralized pagination
- [ ] `infrastructure/hooks/useSearchDebounce.ts` — debounced search
- [ ] `infrastructure/hooks/useLoadingState.ts` — coordinated loading state
- [ ] `infrastructure/utils/formatters.ts` — date + currency + numeric formatters
- [ ] `lib/constants.ts` — polling interval, page size, and all other named constants
- [ ] Playwright auth fixture for each user role
- [ ] CI configuration

**Additionally, for staff-facing apps (add to Foundation bead):**
- [ ] `infrastructure/hooks/useIdleTimeout.ts` — idle timer with warning
- [ ] `components/ui/IdleWarningModal.tsx` — countdown + stay-logged-in action
- [ ] `infrastructure/state/notificationStore.ts` — in-memory notification list
- [ ] `components/ui/NotificationCenter.tsx` — bell icon + badge + dropdown panel
- [ ] `components/ui/NotificationItem.tsx` — individual notification row
- [ ] `infrastructure/hooks/useBadgeCounts.ts` — nav badge count hook

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

## Idle Timeout (Staff-Facing Apps)

Applies to staff admin consoles. Field worker apps and consumer-facing apps are explicitly exempt — see exemption rules below.

**Why not short JWT expiry:** Short JWT TTL causes hard logout with no warning, losing any in-flight form state. That is not acceptable. Idle timeout gives a graceful warning and preserves context.

### Standard Pattern

- Default timeout: **30 minutes** of inactivity. App's vision document may specify a different value.
- **Warning:** A modal appears **5 minutes before** forced logout. Staff can click "Stay logged in" to reset the timer.
- The warning modal preserves all open tabs, form state, and filters — nothing is lost during the warning window.
- After the warning countdown expires with no interaction: session is terminated, user is redirected to login, and a message explains why.
- Activity resets the timer: any keypress, mouse move, or click counts as activity.

### Implementation

Each app that requires idle timeout implements:
- `infrastructure/hooks/useIdleTimeout.ts` — timer logic, activity detection, warning trigger
- `components/ui/IdleWarningModal.tsx` — the warning modal with countdown

Timeout duration is read from a named constant in `lib/constants.ts`:
```ts
export const IDLE_TIMEOUT_MS = 30 * 60 * 1000;       // 30 minutes
export const IDLE_WARNING_BEFORE_MS = 5 * 60 * 1000;  // warn 5 min before
```

### Exemptions

- **Field worker apps** (e.g., TrashTech Driver): Exempt. Active use mid-route — forced logout would be a UX disaster.
- **Consumer-facing apps**: Exempt. Short JWT TTL is appropriate for consumer sessions.

Each app's vision document must explicitly state whether idle timeout applies.

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

## Notification System

**Browser notifications are prohibited.** No app on this platform calls `Notification.requestPermission()` or `new Notification(...)`. The platform controls the notification experience — not the browser, not the OS.

**Reason:** Browser notifications hand control to the browser and the user's OS settings. They appear outside the app, are styled by the OS, and their timing and display are outside our control. Platform notifications are rendered inside the app, styled consistently, and always visible in the notification history.

### Two Channels — Both Used

**Channel 1: Toast (transient)**
Ephemeral alerts. 4-second auto-dismiss. Max one visible at a time. Rules defined in Toast vs Modal Threshold section. Used for: action succeeded, background process finished, non-critical status change.

**Channel 2: In-App Notification Center (persistent)**
A bell icon in the top bar with a count badge. Platform alerts accumulate here and remain until dismissed. Used for: anything important enough that a missed toast would be a problem.

**Why both:** A 4-second toast can be missed. The notification center ensures nothing falls through. Staff can look at the bell at any time and see everything they may have missed.

### Notification Center — Standard Pattern

Every staff-facing app implements a notification center in its top bar:

- Bell icon with numeric badge (badge hides at zero)
- Clicking bell opens a dropdown panel listing all unread notifications
- Each notification: icon by severity, title, short description, timestamp (always with time — see Formatter Standards), dismiss button
- "Clear all" action at top of panel
- Notifications are ordered newest-first
- Unread notifications are visually distinct from read ones
- Severity levels: `info` (blue), `warning` (amber), `error` (red)

**State:** Notification list lives in a `notificationStore` (Zustand, in-memory — not persisted to localStorage). On page refresh the notification center clears. Persistence to backend API is a deferred decision.

### What Belongs in the Notification Center

Each app's vision document defines which system events generate a notification. Examples for TCP UI: tenant goes past due, service health degrades, billing run completes. Examples for TrashTech Dispatcher (Phase 2): driver misses a stop, route unassigned.

**Rule:** Never create a notification that requires no action and would never interest the user. Notification fatigue reduces trust in the system.

### ESLint Enforcement

`no-browser-notifications` rule blocks `new Notification(...)` and `Notification.requestPermission()` anywhere in the codebase. Violations fail the build.

### Foundation Bead — Notification Requirements

Apps that include a notification center (all staff apps) add to their Foundation bead:
- [ ] `infrastructure/state/notificationStore.ts` — Zustand store, in-memory
- [ ] `components/ui/NotificationCenter.tsx` — bell + badge + dropdown panel
- [ ] `components/ui/NotificationItem.tsx` — individual notification row

---

## Real-Time Updates

**MVP standard: TanStack Query polling.**
- Use `refetchInterval` in TanStack Query for live-updating data.
- The polling interval is defined as a named constant in `lib/constants.ts` — never hardcoded in a component.
- Never use raw `setInterval` for data polling. Use TanStack Query's `refetchInterval` exclusively.

**Phase 2 trigger for WebSocket/SSE:** If polling creates visible lag at scale (many concurrent users/entities), upgrade to WebSocket or SSE. The threshold and trigger condition are defined in the app's vision document.

---

## Deferred Decisions

These items are deliberately deferred — not unresolved, but not needed until the trigger condition is met. Do not open discussion on these until the trigger is reached.

| Item | Trigger to revisit |
|------|--------------------|
| Standard app scaffold (starter template repo new apps fork from) | When second app team begins onboarding |
| Shared component package (`packages/ui/`) vs per-app copy | When copy-divergence becomes a maintenance problem across 2+ apps |
| WebSocket / SSE upgrade for real-time data | When TanStack Query polling creates visible lag at production scale |

---

## Registered App Vision Documents

Each app team registers their vision document here when onboarding to the platform.

| App | Vision Document | Registered |
|-----|----------------|-----------|
| Tenant Control Plane UI | `docs/frontend/TCP-UI-VISION.md` (this repo) | 2026-02-20 |
| TrashTech Pro | `docs/apps/trashtech/VISION.md` | 2026-02-20 |

**To register a new app:** Submit a change request to the platform orchestrator (see DOC-REVISION-STANDARDS.md → Governance). Add a row to this table with the app name, vision doc path, and date.

---

## Decision Log

Standards decisions that are settled. Agents must not re-open these without an explicit user directive. Rationale includes what was considered and rejected.

| Date | Decision | Rationale (includes what was NOT chosen) | Decided By |
|------|----------|------------------------------------------|-----------|
| 2026-02-20 | Next.js (App Router) + TypeScript as the standard frontend framework | Mature ecosystem, strong TypeScript support, server components, BFF routes built in. Rejected: Vite + React (no BFF), Remix (less adoption), SvelteKit (agent unfamiliarity). | User + Orchestrator |
| 2026-02-20 | CSS custom properties in `globals.css` as the token system — not Tailwind config values directly | CSS variables cascade, can be overridden per component or theme without rebuilding Tailwind. Rejected: hardcoding Tailwind classes in components (no single source of truth). | Platform Orchestrator |
| 2026-02-20 | Semantic colors shared across apps — brand palette app-specific | Semantic meaning (success=green, danger=red) must be consistent platform-wide. Brand color is per-product identity. Rejected: fully unified visual theme across all apps (ignores product identity). | User + TrashTech Orchestrator |
| 2026-02-20 | Tab system is desktop-only — mobile views use stack navigation | Tabs are a desktop interaction pattern. Mobile users navigate with back button + flat drill-down. Rejected: responsive tab system (too complex, poor mobile UX). | TrashTech Orchestrator (confirmed) |
| 2026-02-20 | ESLint rules enforced from day one — violations fail build, no override comments | Consistency by tooling not discipline. If rules can be bypassed in emergencies, they will be bypassed routinely. Rejected: lint warnings (ignored), convention-based standards (drift over time). | Platform Orchestrator |
| 2026-02-20 | TanStack Query `refetchInterval` for real-time data — WebSocket/SSE deferred to Phase 2 | Polling is sufficient for MVP, simpler to implement and debug. WebSocket adds operational complexity. Rejected: WebSocket from day one (over-engineering for initial scale). | TrashTech Orchestrator |
| 2026-02-20 | Playwright E2E against real backend only — no mocking, no MSW, no stubs | Tests that mock the backend don't catch backend contract changes. Real integration is the only valid proof. Rejected: MSW mocking (fast but doesn't catch real failures). | User |
| 2026-02-20 | Document is product-agnostic — all app-specific content belongs in each app's vision doc | Platform doc must be usable by any future app team without mental search-and-replace. Rejected: including TrashTech-specific examples inline (confuses future app teams). | User |
| 2026-02-20 | Platform standards are single source of truth — app teams follow, do not fork or restate | Parallel versions drift and contradict each other. If TrashTech restates a rule, agents don't know which version to follow. Rejected: each app maintains its own copy of shared rules. | User |
| 2026-02-20 | Changes to platform docs require a formal change request through the platform orchestrator | Uncontrolled edits by app teams would make the doc an unreliable source. Rejected: any orchestrator can edit any doc (no ownership = no accountability). | User |
| 2026-02-20 | Browser notifications are prohibited on all platform apps — platform notifications only | Browser notifications hand UX control to the OS and browser settings. Platform notifications (toast + notification center) are styled consistently and always visible in history. Rejected: `Notification.requestPermission()` / `new Notification(...)` — outside platform control. | User |
| 2026-02-20 | Notification system uses two channels: toast (transient) + in-app notification center (persistent) | Toast alone can be missed in 4 seconds. Notification center ensures nothing is lost. Both are needed — one for immediacy, one for history. Rejected: toast only (missed alerts gone forever), notification center only (no immediate feedback). | User |
| 2026-02-20 | Idle timeout is a standardized pattern for staff-facing apps — 30 min default, 5 min warning | Staff consoles with destructive actions need session expiry. Short JWT TTL alone causes hard logout with no warning, losing form state. Rejected: JWT TTL as sole mechanism (no graceful warning, loses in-flight work). Field worker and consumer apps are explicitly exempt. | Platform Orchestrator + TrashTech Orchestrator |
| 2026-02-20 | Nav badge counts are a standardized pattern — hook signature and placement standardized, data source app-specific | Any sidebar nav benefits from counts. Standardizing the pattern prevents three different badge implementations. Rejected: each app invents its own badge system (divergence over time). | Platform Orchestrator + TrashTech Orchestrator |
| 2026-02-20 | Standard hooks (useMutationPattern, useQueryInvalidation, usePagination, useSearchDebounce, useLoadingState) standardized as pattern docs — no shared package | Every TanStack Query app needs these. Pattern docs prevent divergence without shared package versioning overhead. Each app implements locally. Rejected: shared npm package (versioning overhead, monorepo coupling). | Platform Orchestrator + TrashTech Orchestrator |
| 2026-02-20 | Date and currency formatting rules standardized — each app implements local formatters.ts following the rules | "Feb 20" vs "February 20th" vs "02/20" across apps looks broken. Consistent format across the platform builds trust. Rejected: each app formats dates independently (guaranteed drift). | Platform Orchestrator + TrashTech Orchestrator |
| 2026-02-20 | Re-authentication for sensitive actions is NOT standardized — each app decides | Risk profiles differ: terminating a tenant (catastrophic, unrecoverable) vs. canceling a route (recoverable). A single standard would over-engineer low-risk apps or under-protect high-risk ones. Revisit when two apps both implement re-auth and can extract a pattern. | Platform Orchestrator + TrashTech Orchestrator |
| 2026-02-20 | Breadcrumbs rejected for this platform | Neither current app has hierarchy deep enough to warrant breadcrumbs. A 2-level drill-down with a breadcrumb trail is noise. Rejected: breadcrumbs as a navigation aid (no hierarchy to navigate). | TrashTech Orchestrator + Platform Orchestrator |
| 2026-02-20 | Sidebar favorites rejected | Current apps have 4–5 flat nav items. Favorites are for apps with 20+ nav destinations. Adds UI complexity with no payoff at our scale. Rejected: favorites pinning (wrong scale). | TrashTech Orchestrator + Platform Orchestrator |
| 2026-02-20 | Global popup manager (third notification channel) rejected | Toast = transient, modal = blocking. Two channels covers every case. A third channel creates a decision overhead question agents should never have to answer. Rejected: queued popup system separate from toast and modal. | TrashTech Orchestrator + Platform Orchestrator |

---

> **Revision History** is at the top of this document (immediately after the header). See `docs/frontend/DOC-REVISION-STANDARDS.md` for the standards governing how this document is maintained.
