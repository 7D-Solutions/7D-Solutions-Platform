# Platform Frontend — Foundation Bead Requirements

> **Who reads this:** Any agent implementing the Foundation bead for a new platform app.
> **What it covers:** The complete checklist of what a Foundation bead must deliver, the Infrastructure Map requirement, and testing standards.
> **Rule:** No feature bead ships until every item on the Foundation checklist is complete and verified. Features built without the Foundation break under each other.

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | Platform Orchestrator | Extracted from PLATFORM-FRONTEND-STANDARDS.md rev 1.8. Foundation bead checklist (all apps + staff apps + mobile apps), Infrastructure Map requirement, testing standards. Decision Log populated from master. |

---

## Foundation Bead — Purpose

The Foundation bead establishes every shared system an app needs before a single feature screen can be built correctly. Without it:
- Feature agents pick arbitrary state patterns (no stores yet)
- Feature agents make styling decisions that conflict with each other (no tokens yet)
- Feature agents build their own button variants, their own modals (no component library yet)
- Tests have no auth fixture to build on

The Foundation bead is the contract all feature beads depend on. It is always the first bead in the dependency graph.

---

## Checklist — All Apps

Every platform app's Foundation bead must deliver all of the following. A Foundation bead is not done until every checkbox is verified.

### Design System
- [ ] `app/globals.css` — full CSS token system (port from `docs/reference/fireproof/src/styles/tokens.css`, adapt brand values)
- [ ] `tailwind.config.ts` — configured to reference CSS custom properties as named tokens. No raw hex values in config.

### Component Library (`components/ui/`)
- [ ] `Button.tsx` — all variants (primary, secondary, success, danger, warning, info, ghost, outline), all sizes (compact/xs/sm/md/lg/xl), double-click protection ON by default, loading state with spinner, icon support, active state
- [ ] `StatusBadge.tsx` — platform status config map, audience prop support, icon support, compact/default/large variants, app-specific status extension pattern
- [ ] `Modal.tsx` — all behaviors: no backdrop-click close, Escape key close, portal rendering, two close behaviors (onClose / onFullClose), all sizes (sm/md/lg/xl), composition pattern (Modal.Body, Modal.Actions)
- [ ] `ViewToggle.tsx` — row/card toggle, preference persisted per user per table to backend API
- [ ] `DataTable.tsx` — column manager built in (show/hide, drag-reorder, backend-persisted, tab-scoped, reset to default)
- [ ] All form components: `FormInput`, `NumericFormInput`, `FormSelect`, `FormTextarea`, `FormCheckbox`, `Checkbox`, `FormRadio`, `SearchableSelect`, `DateRangePicker`, `FileInput`
- [ ] `components/ui/index.ts` — single import point for all UI components

### State Management (`infrastructure/`)
- [ ] `infrastructure/state/tabStore.ts`
- [ ] `infrastructure/state/modalStore.ts`
- [ ] `infrastructure/state/useFormStore.ts`
- [ ] `infrastructure/state/useFilterStore.ts`
- [ ] `infrastructure/state/useSearchStore.ts`
- [ ] `infrastructure/state/useUploadStore.ts`
- [ ] `infrastructure/state/useSelectionStore.ts`
- [ ] `infrastructure/state/useViewStore.ts`
- [ ] `infrastructure/state/notificationStore.ts`

### Standard Hooks (`infrastructure/hooks/`)
- [ ] `useBeforeUnload.ts` — browser close warning when form is dirty
- [ ] `useColumnManager.ts` — column visibility, order, and persistence
- [ ] `useMutationPattern.ts` — standardized API mutations
- [ ] `useQueryInvalidation.ts` — standardized cache invalidation
- [ ] `usePagination.ts` — centralized pagination state and navigation
- [ ] `useSearchDebounce.ts` — debounced search input
- [ ] `useLoadingState.ts` — coordinated loading across concurrent operations

### Services (`infrastructure/services/`)
- [ ] `userPreferencesService.ts` — backend API calls for persisting column config and view preferences

### Utilities (`infrastructure/utils/`)
- [ ] `formatters.ts` — all formatters per `PLATFORM-LANGUAGE.md` rules: `formatDate`, `formatDateTime`, `formatCurrency`, `formatPercent`, `formatNumber`

### Constants (`lib/constants.ts`)
All named constants defined here before any feature bead uses them:
- [ ] `PAGINATION_DEFAULT_PAGE_SIZE` — default page size for all tables
- [ ] `SEARCH_DEBOUNCE_MS` — default debounce delay for search inputs
- [ ] `TOAST_DURATION_MS` — 4000 (4 seconds)
- [ ] `REFETCH_INTERVAL_MS` — TanStack Query polling interval

### ESLint (`eslint-local-rules/`)
- [ ] All custom rules active and configured (see `PLATFORM-STATE.md` → ESLint Enforcement)
- [ ] Verified: `npm run lint` passes with zero violations on the scaffold

### Infrastructure Map
- [ ] `INFRASTRUCTURE_MAP.md` in `infrastructure/` directory — complete and accurate (see below)

### Auth
- [ ] Staff JWT auth middleware configured (httpOnly cookie, role enforcement at middleware level)
- [ ] BFF layer scaffolded — all data calls route through Next.js API routes, browser never calls backend directly

### Testing
- [ ] Playwright auth fixture for each user role in the app (e.g., `loginAsStaff()`)
- [ ] At least one Playwright smoke test: login → navigate to primary section → verify page loads

### CI
- [ ] CI configuration — build and Playwright test run added alongside existing CI jobs

---

## Checklist Additions — Staff-Facing Apps

Staff admin consoles add the following to the Foundation bead (in addition to the checklist above):

### Idle Timeout
- [ ] `infrastructure/hooks/useIdleTimeout.ts` — 30-minute default, 5-minute warning, activity detection
- [ ] `components/ui/IdleWarningModal.tsx` — countdown + "Stay logged in" button, preserves form state

### Notification Center
- [ ] `components/ui/NotificationCenter.tsx` — bell icon, badge, dropdown panel (newest-first, severity icons, timestamps, dismiss each, clear all)
- [ ] `components/ui/NotificationItem.tsx` — individual notification row

### Nav Badge Counts
- [ ] `infrastructure/hooks/useBadgeCounts.ts` — returns `Record<navKey, number>`, data source defined by the app's vision doc

---

## Checklist Additions — Mobile / Field Worker Apps

Mobile-first apps add the following to the Foundation bead (in addition to the all-apps checklist above):

- [ ] `infrastructure/hooks/useNetworkStatus.ts` — online/offline detection
- [ ] `infrastructure/services/mutationQueue.ts` — IndexedDB mutation queue for offline operations
- [ ] `infrastructure/services/syncOnReconnect.ts` — processes queued mutations on reconnect
- [ ] Service worker configuration — caches critical data for offline app open
- [ ] `components/ui/BottomNav.tsx` — bottom navigation bar (max 5 items, badge count support)
- [ ] Automated touch-target test — Playwright test verifying every interactive element meets 48×48px CSS minimum

---

## Infrastructure Map (Required in Every App)

Every app must maintain an `INFRASTRUCTURE_MAP.md` in its `infrastructure/` directory. This is **the first document an agent reads** before touching any UI code.

### What it must contain

- Every centralized system with: file path, purpose, and a usage example
- A quick-reference table: "If you need to do X, use Y, import from Z"
- Updated every time new infrastructure is added to the app

### What it replaces

Without an Infrastructure Map, agents search the codebase randomly for patterns, find old or incorrect examples, and re-implement things that already exist. The Infrastructure Map eliminates this.

### Reference

`docs/reference/fireproof/src/infrastructure/INFRASTRUCTURE_MAP.md` — the Fireproof implementation. Port this structure (not the content) into every new app.

---

## Testing Standards

- **Playwright E2E only** — no unit tests for UI behavior. Unit tests catch implementation details, not user-visible failures.
- **Real backend** — no mocking, no MSW, no stubs, no fake API responses. Tests that mock the backend don't catch backend contract changes. Real integration is the only valid proof.
- **loginAs fixture** — each app defines a Playwright fixture for each user role: `loginAsStaff()`, `loginAsDriver()`, `loginAsCustomer()`. Every spec that needs auth uses the fixture — never hardcodes credentials.
- **Coverage requirement:** Every screen has a Playwright spec covering at minimum: login → navigation → at least one real data read (or empty state display).
- **CI behavior:** E2E tests are skipped if the backend is not available (environment variable gate). Never fake the backend to make CI green.

---

## Open Questions

Do not create beads until these are resolved.

| # | Question | Status |
|---|----------|--------|
| — | No open questions at this time. | — |

---

## Decision Log

Decisions specific to Foundation and testing. Do not re-open without an explicit user directive.

| Date | Decision | Rationale (includes what was NOT chosen) | Decided By |
|------|----------|------------------------------------------|-----------|
| 2026-02-20 | Playwright E2E against real backend — no mocking, no MSW, no stubs | Tests that mock the backend do not catch backend contract changes. Real integration is the only valid proof. Rejected: MSW mocking (fast but doesn't catch real failures), unit testing UI behavior (catches implementation details, misses integration failures). | User |
| 2026-02-20 | Infrastructure Map required in every app's Foundation bead | Without it, agents search randomly, find old examples, and re-implement existing infrastructure. The map eliminates this waste on every subsequent bead. Rejected: relying on code comments and tribal knowledge (doesn't survive agent context turnover). | Platform Orchestrator |
| 2026-02-20 | No feature bead ships until Foundation checklist is complete | Features built without the Foundation use local state, ad-hoc styling, and conflicting patterns. Fixing this retroactively costs more than doing it first. Rejected: building features in parallel with Foundation (produces work that must be redone). | Platform Orchestrator |
| 2026-02-20 | All named constants in `lib/constants.ts` — no magic numbers in components | A polling interval hardcoded as `5000` in three components means three places to update when it changes. Named constants mean one. Rejected: inline constants (guaranteed drift and missed updates). | Platform Orchestrator |

---

> See `docs/frontend/DOC-REVISION-STANDARDS.md` for the standards governing how this document is maintained.
