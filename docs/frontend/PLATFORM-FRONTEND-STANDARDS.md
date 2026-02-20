# 7D Solutions Platform — Frontend Standards Index

> **Who reads this:** Every agent starting work on any platform frontend app.
> **Start here. Then go to the document for your specific topic.**
> This file covers the principles, architecture, and cross-cutting standards that apply to every app. Topic-specific rules live in their own documents.

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | Platform Orchestrator | Created document. Guiding principles, tech stack, repository structure, CSS token system, Button/StatusBadge/Modal/DataTable/Form components, tab system, unsaved changes, state management, ESLint enforcement, language standards, navigation standards, testing standards, infrastructure map requirement, foundation bead checklist, app vision documents table. |
| 1.1 | 2026-02-20 | Platform Orchestrator | Added Multi-Audience Apps section and Mobile Standards section. Incorporated TrashTech Orchestrator input. |
| 1.2 | 2026-02-20 | Platform Orchestrator | Added Toast vs Modal Threshold section and Real-Time Updates section. Added customer-facing language rules. |
| 1.3 | 2026-02-20 | Platform Orchestrator | Genericized per user directive: removed all product-specific content. Belongs in each app's vision doc. |
| 1.4 | 2026-02-20 | Platform Orchestrator | Added Revision History, Decision Log, pointer to DOC-REVISION-STANDARDS.md. Adopted cross-app doc standard. Moved to docs/frontend/ subfolder. |
| 1.5 | 2026-02-20 | Platform Orchestrator | Replaced all absolute paths with repo-relative symlink paths. No content changes. |
| 1.6 | 2026-02-20 | Platform Orchestrator | Changed By and Decided By: replaced agent names with roles throughout. |
| 1.7 | 2026-02-20 | Platform Orchestrator | Formalized governance model. Converted Open Items to Deferred Decisions. Genericized App Vision Documents to a registry. |
| 1.8 | 2026-02-20 | Platform Orchestrator | Added Notification System, Standard Hooks, Formatter Standards, Idle Timeout, Nav Badge Counts. ESLint prohibition on browser Notification API. Updated Foundation Bead checklist. 9 new Decision Log entries. |
| 1.9 | 2026-02-20 | Platform Orchestrator | Split into 6 topic files. This file is now the index. Content moved to PLATFORM-COMPONENTS.md, PLATFORM-STATE.md, PLATFORM-LANGUAGE.md, PLATFORM-NOTIFICATIONS.md, PLATFORM-MOBILE.md, PLATFORM-FOUNDATION.md. Cross-cutting standards retained here. |

---

## Document Map — Read This First

| Document | Read it when you are... |
|----------|------------------------|
| **This file** | Starting any frontend work. Orientation, principles, architecture, cross-cutting rules. |
| `PLATFORM-COMPONENTS.md` | Building any UI screen or component. CSS tokens, Button, StatusBadge, Modal, DataTable, forms. |
| `PLATFORM-STATE.md` | Writing state, mutations, API calls, pagination, or search. Zustand stores, standard hooks, ESLint rules. |
| `PLATFORM-LANGUAGE.md` | Writing labels, display values, errors, dates, or currency. Language rules and formatter standards. |
| `PLATFORM-NOTIFICATIONS.md` | Implementing any alert, status message, confirmation, or event notification. Toast, notification center, browser notifications prohibited. |
| `PLATFORM-MOBILE.md` | Building a mobile-first app or an app serving multiple audiences (staff, customers, field workers). |
| `PLATFORM-FOUNDATION.md` | Implementing the Foundation bead for a new app. Complete checklist, Infrastructure Map requirement, testing standards. |

**New to this platform?** Read this file first, then `PLATFORM-COMPONENTS.md`, then `PLATFORM-STATE.md`. The other files are reference — read them when your work touches their topic.

---

## Governance — Who Can Change What

**Platform Orchestrator** owns all documents in `docs/frontend/`. No other agent edits these documents without authorization.

**App teams** (TrashTech Orchestrator, future app teams) read these documents and implement what they say. They do not maintain parallel versions of platform rules in their own repos.

### Change Request Process

When an app team needs a platform standard changed, they send a mail to the platform orchestrator with:
- Subject: `Standards change request — [doc name] — [short description]`
- Current rule (exact quote from the doc)
- Proposed rule
- Reason the current rule does not work
- Which beads in their project are blocked without the change

The platform orchestrator approves or rejects. If approved, the orchestrator commits the change. The app team implements after the platform commit is confirmed.

**App teams never modify platform docs directly** — not even for typo fixes. Submit a change request.

---

## Guiding Principles

### Minimum Clicks, Maximum Clarity
Reduce the number of steps to complete any task — but never at the expense of clarity or ease of use. Three clicks that feel natural beat two clicks that are confusing.

### Built for Everyone
Apps must be usable by non-technical staff, customers, and field workers without documentation. If a label needs explanation, it is the wrong label.

### Context Before Action
Users see full context before taking any action. Related information is accessible without navigating away from the current screen.

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

> **Mobile:** The standard stack applies to desktop/responsive apps. Mobile-first apps use the same stack with distinct constraints — see `PLATFORM-MOBILE.md`.

---

## Repository Structure

Each app is self-contained within the platform monorepo:

```
apps/
  tenant-control-plane-ui/     # TCP UI — staff admin console
    app/                       # Next.js App Router pages + BFF routes
    components/
      ui/                      # CENTRALIZED — all shared components live here
    infrastructure/
      state/                   # All Zustand stores
      hooks/                   # Standard hooks (useMutationPattern, etc.)
      services/                # userPreferencesService, etc.
      utils/                   # formatters.ts, cn.ts, logger.ts
      INFRASTRUCTURE_MAP.md    # First document agents read — maps everything
    lib/
      api/                     # BFF client wrappers, types, TanStack Query keys
      server/                  # Server-only utilities (bffFetch, auth guards)
      constants.ts             # All named constants — no magic numbers in components
    src/
      styles/                  # globals.css (design tokens)
  trashtech-pro/               # TrashTech — structure defined in TrashTech vision doc
```

**Rule:** No app imports from another app's directory. Shared platform logic lives in the platform backend, not in frontend app code.

---

## Navigation Standards

**Flat over nested.** Navigation menus have one level. No dropdowns inside dropdowns.

**Minimum clicks target:**
- 1 click to reach a section
- 1 more click to reach a specific record
- Actions available on the record — no extra navigation

**Tabs on detail pages** instead of separate pages for sub-sections. All information about a record is accessible via tabs on its detail page.

**Nav badge counts:** Nav items may display a numeric count badge. Badge appears top-right of the nav item. Zero count hides the badge. Each app implements a `useBadgeCounts` hook returning `Record<navKey, number>`. Data source is app-specific.

**Breadcrumbs:** Not used on this platform. Current navigation hierarchies (max 2 levels) do not warrant them.

---

## Tab System

Apps use a browser-tab-like interface. Users can have multiple records open simultaneously without losing context.

**Tab behaviors:**
- **Preview tabs** (shown in italics): browsing, viewing — replaced by the next navigation
- **Permanent tabs**: creating, actively editing — stay until explicitly closed
- Tabs **persist across browser refresh** (Zustand + localStorage)
- Tab state is **scoped**: each tab has its own form data, filters, search, column config, and open modals
- **Split view**: two tabs side by side with a draggable divider (20%–80% range)
- Tabs are drag-to-reorderable
- Right-click context menu: Close / Close Others / Close All
- Closing a tab with unsaved changes opens a confirmation modal listing the unsaved fields

Reference: `docs/reference/fireproof/src/infrastructure/components/TabManager/` and `docs/reference/fireproof/src/infrastructure/state/tabStore.ts`

> **Mobile:** Tab system is **desktop-only**. Mobile views use stack navigation. See `PLATFORM-MOBILE.md`.

---

## Unsaved Changes Protection

Two layers — both required on any screen with a form.

**Layer 1 — Browser close warning (`useBeforeUnload`):**
When a form is dirty, attempting to close the browser tab or window triggers a native browser warning. Disabled during E2E testing via `VITE_DISABLE_UNLOAD_WARNING=true`.

**Layer 2 — Unsaved Changes Panel:**
A collapsible panel shown on the page when the form is dirty. Field-by-field diff: field name, "Was:" value, "Now:" value. Users see exactly what they would lose.

Reference: `docs/reference/fireproof/src/infrastructure/hooks/useBeforeUnload.ts` and `docs/reference/fireproof/src/infrastructure/components/UnsavedChangesPanel.tsx`

---

## Idle Timeout (Staff-Facing Apps)

Applies to staff admin consoles. Field worker apps and consumer apps are explicitly exempt.

- Default: **30 minutes** of inactivity. App's vision document may specify a different value.
- **Warning:** Modal appears **5 minutes before** forced logout. Staff click "Stay logged in" to reset.
- Warning modal preserves all open tabs, form state, and filters — nothing is lost.
- After warning expires: session terminated, redirect to login, plain-English explanation.

**Why not short JWT TTL:** Hard logout with no warning loses in-flight form state.

See `PLATFORM-FOUNDATION.md` for Foundation bead checklist items.

---

## Real-Time Updates

**Standard: TanStack Query polling.**
- Use `refetchInterval` — never raw `setInterval` for data polling.
- Interval defined as a named constant in `lib/constants.ts` → `REFETCH_INTERVAL_MS`. Never hardcoded in a component.

**Upgrade trigger for WebSocket/SSE:** If polling creates visible lag at scale, upgrade. Trigger condition defined in each app's vision document.

---

## Deferred Decisions

These items are deliberately deferred — not unresolved, but not needed until the trigger condition is met. Do not open discussion until the trigger is reached.

| Item | Trigger to revisit |
|------|--------------------|
| Standard app scaffold (starter template new apps fork from) | When second app team begins onboarding |
| Shared component package (`packages/ui/`) vs. per-app copy | When copy-divergence becomes a maintenance problem across 2+ apps |
| WebSocket / SSE upgrade for real-time data | When TanStack Query polling creates visible lag at production scale |
| Notification center persistence to backend API | When in-memory loss on page refresh becomes a real user complaint |
| Re-authentication standard for sensitive actions | When two or more apps both implement re-auth and a common pattern emerges |

---

## Registered App Vision Documents

Each app team registers their vision document here when onboarding.

| App | Vision Document | Registered |
|-----|----------------|-----------|
| Tenant Control Plane UI | `docs/frontend/TCP-UI-VISION.md` (this repo) | 2026-02-20 |
| TrashTech Pro | `docs/apps/trashtech/VISION.md` | 2026-02-20 |

**To register a new app:** Submit a change request (see Governance above). Add a row with app name, vision doc path, and date.

---

## Open Questions

| # | Question | Status |
|---|----------|--------|
| — | No open questions at this time. | — |

---

## Decision Log

Platform-wide governance and architectural decisions. Topic-specific decisions live in each topic document's own Decision Log. Do not re-open these without an explicit user directive.

| Date | Decision | Rationale (includes what was NOT chosen) | Decided By |
|------|----------|------------------------------------------|-----------|
| 2026-02-20 | Next.js (App Router) + TypeScript as the standard frontend framework | Mature ecosystem, strong TypeScript support, server components, BFF routes built in. Rejected: Vite + React (no BFF), Remix (less adoption), SvelteKit (agent unfamiliarity). | User + Platform Orchestrator |
| 2026-02-20 | Platform standards are product-agnostic — all app-specific content lives in each app's vision doc | Platform doc must be usable by any future app team without mental search-and-replace. Rejected: including app-specific examples inline. | User |
| 2026-02-20 | Platform standards are single source of truth — app teams follow, do not fork or restate | Parallel versions drift and contradict each other. Rejected: each app maintains its own copy of shared rules. | User |
| 2026-02-20 | Changes to platform docs require a formal change request through the platform orchestrator | Uncontrolled edits make the doc unreliable. Rejected: any agent can edit any doc. | User |
| 2026-02-20 | TanStack Query `refetchInterval` for real-time — WebSocket/SSE deferred | Polling is sufficient for MVP, simpler to implement and debug. Rejected: WebSocket from day one (over-engineering). | TrashTech Orchestrator |
| 2026-02-20 | Idle timeout standardized as a pattern for staff-facing apps — field worker and consumer apps exempt | Staff consoles with destructive actions need session expiry. Short JWT TTL loses form state with no warning. Rejected: JWT TTL as sole mechanism, universal idle timeout for all app types. | Platform Orchestrator + TrashTech Orchestrator |
| 2026-02-20 | Nav badge counts standardized — hook signature and placement standardized, data source app-specific | Any sidebar nav benefits from counts. Standardizing prevents divergent implementations. Rejected: each app invents its own badge system. | Platform Orchestrator + TrashTech Orchestrator |
| 2026-02-20 | Re-authentication for sensitive actions NOT standardized — each app decides | Risk profiles differ enough across apps. Revisit when two apps both implement it and a common pattern emerges. | Platform Orchestrator + TrashTech Orchestrator |
| 2026-02-20 | Breadcrumbs rejected for this platform | Neither current app has hierarchy deep enough to warrant breadcrumbs. Noise on 2-level navigation. | TrashTech Orchestrator + Platform Orchestrator |
| 2026-02-20 | Sidebar favorites rejected | Current apps have 4–5 flat nav items. Favorites are for apps with 20+ destinations. Adds complexity with no payoff. | TrashTech Orchestrator + Platform Orchestrator |
| 2026-02-20 | Platform split into 6 topic documents — this file is the index | Single large file is hard to navigate. Topic files let agents read only what they need. Rejected: single monolithic standards file (too long, agents read irrelevant sections). | User |

---

> See `docs/frontend/DOC-REVISION-STANDARDS.md` for the standards governing how this document is maintained.
