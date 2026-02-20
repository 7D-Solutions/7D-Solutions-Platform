# Platform Frontend — Components & Design System

> **Who reads this:** Any agent building a UI screen or component.
> **What it covers:** Design tokens, CSS infrastructure, and every shared component an agent must use instead of building their own.
> **Rule:** If a component is listed here, it is the only way to render that thing. No ad-hoc alternatives.

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | Platform Orchestrator | Extracted from PLATFORM-FRONTEND-STANDARDS.md rev 1.8. CSS token system, color system, component size scale, Button, StatusBadge, Modal, DataTable/ViewToggle, form components. Decision Log populated from master. |
| 1.1 | 2026-02-20 | Platform Orchestrator | Added SupportSessionBanner component: non-dismissable banner for active support sessions, polling detection pattern, termination flow, constants. Required for all apps supporting tech support sessions. |

---

## CSS Infrastructure — Design Tokens

All design values are defined as CSS custom properties in `app/globals.css`. Tailwind is configured to reference these variables. **Nothing is hardcoded in components.** No raw hex values, no arbitrary Tailwind classes like `bg-[#2c72d5]`.

Port from: `docs/reference/fireproof/src/styles/tokens.css` — adapt values as needed but preserve the category structure.

### Color System

```css
:root {
  /* Primary — app-specific brand color */
  --color-primary: #2c72d5;        /* TCP UI blue — TrashTech uses forest green */
  --color-primary-light: #5691e3;
  --color-primary-dark: #1e5bb8;

  /* Semantic — SHARED across all apps, never overridden */
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

**Shared vs. app-specific:**
- **Semantic colors** (`--color-success`, `--color-warning`, `--color-danger`, `--color-info`, text, backgrounds, borders): shared across all apps. Never overridden.
- **Brand palette** (`--color-primary` and any app-specific tokens): app-specific. Each app's vision document defines its brand color.

### Typography, Spacing, Shadows, Z-Index, Transitions

Full token reference: `docs/reference/fireproof/src/styles/tokens.css`

Port all token categories into each app's `globals.css`:
- Typography (font families, sizes, weights, line heights)
- Spacing (0.25rem increment scale)
- Shadows (elevation scale)
- Border radius (named scale)
- Z-index (named layers: dropdown, modal, tooltip, notification)
- Transitions (named duration + easing)

### Component Size System

Shared sizing scale used by Button, Badge, Tag, Input — ensures all interactive elements align visually.

```css
/* Component sizes */
--component-size-compact-padding-y: 5px;  --component-size-compact-min-height: 26px;
--component-size-xs-padding-y: 6px;       --component-size-xs-min-height: 28px;
--component-size-sm-padding-y: 8px;       --component-size-sm-min-height: 32px;
--component-size-md-padding-y: 10px;      --component-size-md-min-height: 38px;
--component-size-lg-padding-y: 12px;      --component-size-lg-min-height: 44px;
--component-size-xl-padding-y: 16px;      --component-size-xl-min-height: 52px;
```

### Layout Tokens

```css
--header-height: 77px;
--tab-bar-height: 48px;
--chrome-total-height: 155px;   /* header + tab bar + margin */
```

---

## Centralized Component Library

**Non-negotiable rule:** No raw `<button>` elements. No `window.confirm()`. No `window.alert()`. No inline status colors. No ad-hoc modal implementations.

Every interactive element is imported from `components/ui/`. ESLint rules in `PLATFORM-STATE.md` enforce this — violations fail the build.

---

### Button

**Two properties define every button:** `variant` (semantic meaning) + `size` (layout context).

#### Variants

| Variant | Color | When to use |
|---------|-------|-------------|
| `primary` | Brand color | Main action on a page (Save, Confirm, Assign) |
| `secondary` | Neutral | Supporting actions (Edit, View, Export) |
| `success` | Green | Positive completion actions |
| `danger` | Red | Destructive actions (Terminate, Delete, Revoke) |
| `warning` | Amber | Caution actions (Suspend, Force-release) |
| `info` | Teal | Informational actions |
| `ghost` | Transparent | Tertiary / low-emphasis actions |
| `outline` | Border only | Alternative secondary style |

#### Sizes

| Size | Min height | When to use |
|------|-----------|-------------|
| `compact` | 26px | Dense toolbars, tight table rows |
| `xs` | 28px | In-table row actions |
| `sm` | 32px | Secondary actions, sidebars |
| `md` | 38px | Default — most page-level actions |
| `lg` | 44px | Primary CTA |
| `xl` | 52px | Prominent confirmation dialogs |

#### Built-in behaviors (all ON by default)

- **Double-click protection:** 1000ms cooldown after click. Prevents duplicate submissions on financial and destructive actions. Cannot be disabled without explicit justification in code comment.
- **Loading state:** `loading={true}` shows a spinner and disables the button. The button manages its own disabled state — caller does not need to do this manually.
- **Icon support:** Optional leading icon via `icon` prop.
- **Active state:** `active` prop for toggleable buttons (e.g., active nav item).

```tsx
// Correct usage
<Button variant="danger" size="sm">Terminate</Button>
<Button variant="primary" size="md" loading={isPending}>Save Changes</Button>

// Never
<button className="bg-red-500 text-white">Terminate</button>
```

---

### StatusBadge

All status rendering goes through `<StatusBadge status="active" />`. Color, label, and icon are determined by the component — never by the calling page.

**Rule:** Never render status inline. Never hardcode a color based on a status string.

```tsx
// Correct
<StatusBadge status="suspended" />
<StatusBadge status="completed" audience="driver" />

// Never
<span className="text-red-500">Suspended</span>
```

#### Variants

- `default` — standard badge
- `compact` — for dense tables
- `large` — for prominent display

#### Audience prop (optional)

`audience="admin"` (default) | `audience="driver"` | `audience="customer"`

The same status key can have different display labels for different user audiences. The `audience` prop selects the right label. When omitted, defaults to `admin`.

#### Platform status types (shared — never removed or recolored per app)

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

#### App-specific status types

Each app extends the platform config with its own statuses. App-specific types are added to a separate `appStatusConfigs` map and merged at app initialization. Platform types are never removed or recolored.

---

### Modal

**Rules:**
- Never use `window.confirm()` or `window.alert()` — always use the `Modal` component
- No backdrop-click close — prevents accidental data loss on destructive actions
- Escape key closes (unless `preventClosing` prop is set)
- Always renders via React portal to `document.body` — never trapped in parent layout
- Z-index managed automatically — nested modals stack correctly
- Size comes from named tokens — never hardcoded pixel widths

#### Two close behaviors

- `onClose` — dismiss/cancel, keep the user in their current context
- `onFullClose` — X button, navigate back to the parent page

#### Sizes

| Size | Width | Use |
|------|-------|-----|
| `sm` | 480px | Simple confirmations, alerts |
| `md` | 600px | Standard forms |
| `lg` | 800px | Complex forms, detail views |
| `xl` | 1000px | Multi-section workflows |

#### Composition pattern

```tsx
<Modal isOpen={isOpen} onClose={onClose} onFullClose={onFullClose} size="md" title="Suspend Tenant">
  <Modal.Body>
    <p>This tenant will lose access to all services immediately.</p>
    <FormTextarea label="Reason for suspension" {...register('reason')} required />
  </Modal.Body>
  <Modal.Actions>
    <Button variant="ghost" onClick={onClose}>Cancel</Button>
    <Button variant="warning" loading={isPending}>Suspend</Button>
  </Modal.Actions>
</Modal>
```

---

### DataTable and ViewToggle

All list screens support two display modes, toggled by the user:

- **Row view** — compact table, more records visible, best for scanning large datasets
- **Card view** — richer per-record display, best for detail at a glance

The toggle is a shared `<ViewToggle />` component. Preference is persisted per table per user to the backend API (cross-device).

#### Column management (built into DataTable)

Every DataTable supports:
- Show/hide individual columns via a toggle panel
- Drag column headers to reorder
- Column configuration persisted to backend API per user — follows them across devices
- Tab-scoped — each open tab has its own column configuration
- "Reset to default" button — restores original column order and visibility

An "edit columns" mode toggles drag handles and checkboxes on the header row. Changes save on exit.

Reference: `docs/reference/fireproof/src/infrastructure/hooks/useColumnManager.ts`

---

### Form Components

Never use raw HTML form elements. Import everything from `components/ui/`.

| Component | Replaces | Notes |
|-----------|---------|-------|
| `FormInput` | `<input type="text">` | Includes label, error display, validation state |
| `NumericFormInput` | `<input type="number">` | Decimal handling, locale-aware |
| `FormSelect` | `<select>` | Includes label, error display |
| `FormTextarea` | `<textarea>` | Includes label, character count |
| `FormCheckbox` | `<input type="checkbox">` in forms | With label, error state |
| `Checkbox` | `<input type="checkbox">` in tables/grids | No label wrapper |
| `FormRadio` | `<input type="radio">` | With label |
| `SearchableSelect` | `<select>` with search | Dropdown with search filter |
| `SearchableCombobox` | Combobox with custom entry | Allows typing a new value |
| `DateRangePicker` | Date range inputs | For audit log filters, billing date ranges |
| `FileInput` | `<input type="file">` | Drag-drop support |

**Form state:** All form field values are managed through `useFormStore` (see `PLATFORM-STATE.md`) — not local `useState`. This ensures form data survives tab switches.

---

## SupportSessionBanner

**Required in every app that supports tech support sessions.** This is not optional — it is the customer's guarantee that they always know when someone else is in their account.

### What it does

When a 7D support agent has an active session in the app, a persistent banner renders at the top of the page — above all other content, including the app's own navigation. It shows who is logged in, why, and when the session expires. The customer can end the session from the banner.

### Rendering rules

- **Non-dismissable.** No close button. No minimize. It renders as long as the session is active.
- **Top of page, always.** Renders via portal to `document.body`, above the app shell. Not inside any layout that could hide it.
- **Disappears automatically** when the session ends (polling detects expiry within 30 seconds).

### What the banner shows

```
┌─────────────────────────────────────────────────────────────────────────┐
│  🔧  Support session active  —  Alex from 7D Solutions is logged in     │
│      Reason: "Help with route scheduling"  ·  Ends at 3:45 PM           │
│                                            [ End Session Now ]           │
└─────────────────────────────────────────────────────────────────────────┘
```

- Support agent's name (from session metadata, not their JWT — the session is stored server-side)
- "from 7D Solutions" — always this exact company name, hardcoded, not configurable
- Reason text — entered by the support agent when starting the session in TCP
- Expiry time — displayed as a clock time ("Ends at 3:45 PM"), not a countdown
- **End Session Now** button — variant `warning`, terminates the session immediately, no confirmation required

### How the app detects an active support session

The app polls `GET /api/support-sessions/active` every 30 seconds via TanStack Query.

```typescript
// In the root layout — runs on every page:
const { data: supportSession } = useQuery({
  queryKey: ['support-session'],
  queryFn: () => fetch('/api/support-sessions/active').then(r => r.json()),
  refetchInterval: SUPPORT_SESSION_POLL_MS,  // from lib/constants.ts — 30000
  staleTime: 0,
});

// Banner renders when data is non-null:
{supportSession && <SupportSessionBanner session={supportSession} />}
```

The BFF route (`/api/support-sessions/active`) calls the platform's identity-auth service with the tenant's JWT to check for active support sessions on that tenant's account. If none: returns `null`. If one exists: returns `{ agent_name, reason, expires_at, session_id }`.

### Terminating a session from the banner

"End Session Now" calls `DELETE /api/support-sessions/{session_id}` via the BFF. The BFF calls the platform to revoke the session token. TanStack Query's next poll (or an immediate invalidation) removes the banner.

### Component file location

`components/ui/SupportSessionBanner.tsx` — part of the Foundation bead for any app supporting tech support sessions.

### Constants

Add to `lib/constants.ts`:
```typescript
export const SUPPORT_SESSION_POLL_MS = 30_000;
```

---

## Open Questions

Do not create beads until these are resolved.

| # | Question | Status |
|---|----------|--------|
| — | No open questions at this time. | — |

---

## Decision Log

Decisions specific to components and design system. Do not re-open without an explicit user directive.

| Date | Decision | Rationale (includes what was NOT chosen) | Decided By |
|------|----------|------------------------------------------|-----------|
| 2026-02-20 | CSS custom properties in `globals.css` as the token system — not Tailwind config values directly | CSS variables cascade and can be overridden per component or theme without rebuilding Tailwind. Rejected: hardcoding Tailwind classes in components (no single source of truth). | Platform Orchestrator |
| 2026-02-20 | Semantic colors shared across all apps — brand palette is app-specific | Semantic meaning (success=green, danger=red) must be consistent platform-wide. Brand color is per-product identity. Rejected: fully unified visual theme across all apps (erases product identity). | User + TrashTech Orchestrator |
| 2026-02-20 | Button double-click protection ON by default (1000ms cooldown) | Prevents duplicate submissions on financial and destructive actions. Rejected: opt-in protection (relies on developers remembering to enable it on important buttons — they don't). | Platform Orchestrator |
| 2026-02-20 | No backdrop-click close on modals | Backdrop clicks are accidental on destructive actions. A user confirming a termination should not lose that work by clicking 3px outside the modal. Rejected: backdrop close (accidental data loss rate is too high). | Platform Orchestrator |
| 2026-02-20 | All status rendering through StatusBadge — no inline status coloring | Consistency enforced by tooling. A feature screen cannot accidentally invent a "kind of red" for a status. Rejected: convention-based coloring (agents create ad-hoc status colors without realizing it). | Platform Orchestrator |
| 2026-02-20 | ViewToggle (row/card) on all list screens, preference persisted | Users have different workflows. Some want dense row view, others prefer card view. Preference saved per user per table. Rejected: single fixed view (optimizes for one workflow at the expense of others). | User |
| 2026-02-20 | Column manager persisted to backend API (cross-device) — not just localStorage | Column layout follows staff across devices. Rejected: localStorage-only persistence (lost when staff switch computers). | Platform Orchestrator |

---

> See `docs/frontend/DOC-REVISION-STANDARDS.md` for the standards governing how this document is maintained.
