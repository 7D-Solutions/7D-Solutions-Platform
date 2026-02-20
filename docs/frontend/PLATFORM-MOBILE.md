# Platform Frontend — Mobile & Multi-Audience Apps

> **Who reads this:** Any agent building a mobile-first app or an app that serves multiple user audiences (staff, customers, field workers).
> **What it covers:** How to structure multi-audience apps, mobile constraints (all enforced), and the offline pattern.
> **Rule:** Mobile constraints listed here are not aspirational. They are enforced. "Aim for 48px targets" means nothing — 48px is the minimum, period.

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | Platform Orchestrator | Extracted from PLATFORM-FRONTEND-STANDARDS.md rev 1.8. Multi-audience app structure, mobile constraints (touch targets, fonts, navigation, orientation, offline), Foundation bead additions for mobile apps. Decision Log populated from master. |

---

## Multi-Audience Apps

Some apps serve multiple distinct audiences with radically different needs — for example: staff dispatchers, field workers, and customers using the same underlying platform.

**Rule:** Each audience is treated as a separate app experience within the platform monorepo. Audiences share the component library but have distinct navigation, layouts, and interaction patterns.

### Audience Tiers

| Audience | Typical needs | Platform features that apply |
|----------|--------------|------------------------------|
| **Staff / Admin (desktop)** | Dense data, multi-record work, full administrative actions | Full standard: tabs, column manager, row/card toggle, modals, notification center, idle timeout |
| **Customer-facing (responsive)** | Simplified flow, max 3–5 screens, plain language, mobile-friendly | Platform components; no tab system, no column manager, no notification center |
| **Field worker (mobile-first)** | One-handed, glanceable, offline tolerance, in-motion use | See Mobile Standards below — distinct constraints apply |

**Rule:** Never serve desktop and mobile-first users in the same Next.js route or layout. Separate routes, separate layouts — shared component library underneath.

Each app's vision document defines which audience tiers it serves and what the navigation model is for each.

---

## Mobile Standards (Field Worker Apps)

Applies to mobile-first apps (e.g., TrashTech Driver, any future field worker apps). These are **not** responsive desktop apps. They are distinct products with their own interaction model. The component library is shared; the constraints are not.

### Core Constraints (enforced — not aspirational)

| Constraint | Rule |
|-----------|------|
| **Touch targets** | 48×48px CSS minimum on every interactive element. No exceptions. Enforced by automated test in Foundation bead. |
| **Form input font size** | 16px minimum. Below 16px triggers iOS auto-zoom on focus — breaks the flow for workers in motion. |
| **Body text** | 14px minimum. Line height minimum 1.5. |
| **Primary navigation** | Bottom navigation bar only. Maximum 5 items. No left sidebar on any mobile screen. |
| **Primary orientation** | Portrait. Must also function in landscape without layout breakage. |
| **Horizontal scrolling** | Never on any mobile screen. |
| **Loading indicators** | Skeleton loaders only on route and stop screens — no spinners. Workers expect to see content, not wait. |
| **One-handed use** | Primary actions reachable without stretching. Bottom-anchored primary buttons. |
| **Tab system** | Not used. Standard stack navigation only. |
| **Column manager** | Not used. Mobile views use fixed, optimized layouts. |
| **Split view** | Not used. |
| **Multi-step modal flows** | Never. Show a full-screen confirmation page instead — see below. |

### Mobile Navigation Model

**Pattern:** Flat drill-down. List view is the home screen. Tap a record → detail view. Back button returns to the list. No browser-tab-like interface. Each app's vision document defines the specific sections and labels.

### Confirmation on Mobile

Never show a modal layered over the active work screen. Modals are disorienting when the user is in motion or on a small screen.

Instead: navigate to a **full-screen confirmation page** that shows exactly what will happen and provides a large, thumb-reachable confirm button. The user presses back to cancel.

### Offline State Management

Apps that require offline tolerance define in their vision document which specific operations work offline vs. require connectivity. Do not add offline support speculatively — only implement what the vision doc specifies.

**Implementation pattern:**

| Layer | How |
|-------|-----|
| Local mutation queue | IndexedDB — mutations that fail due to connectivity are queued and retried |
| Read operations | TanStack Query with `networkMode: 'offlineFirst'` — serves cached data when offline |
| Sync on reconnect | All queued mutations must be **idempotent** — retrying must not produce duplicates |
| App open | Service worker caches critical data — user can open the app and view data immediately without connectivity |

**Rule:** Never queue a mutation that is not idempotent. If the operation cannot safely be retried, it must not be queued — inform the user that connectivity is required.

### Foundation Bead Additions (Mobile Apps)

In addition to the standard Foundation bead checklist in `PLATFORM-FOUNDATION.md`, mobile apps add:

- [ ] `infrastructure/hooks/useNetworkStatus.ts` — online/offline detection
- [ ] `infrastructure/services/mutationQueue.ts` — IndexedDB mutation queue
- [ ] `infrastructure/services/syncOnReconnect.ts` — processes queued mutations on reconnect
- [ ] Service worker configuration — caches critical data for offline app open
- [ ] `components/ui/BottomNav.tsx` — bottom navigation bar (max 5 items)
- [ ] Automated touch-target test — Playwright test verifying 48px CSS minimum on all interactive elements

---

## Open Questions

Do not create beads until these are resolved.

| # | Question | Status |
|---|----------|--------|
| — | No open questions at this time. | — |

---

## Decision Log

Decisions specific to mobile and multi-audience concerns. Do not re-open without an explicit user directive.

| Date | Decision | Rationale (includes what was NOT chosen) | Decided By |
|------|----------|------------------------------------------|-----------|
| 2026-02-20 | Tab system is desktop-only — mobile views use standard stack navigation | Tabs are a desktop interaction pattern. Mobile users navigate with back button and flat drill-down. Rejected: responsive tab system (too complex, poor mobile UX, doesn't map to mobile mental model). | TrashTech Orchestrator (confirmed) |
| 2026-02-20 | Mobile constraints are enforced, not aspirational | "Aim for 48px" is ignored. Automated tests verify it. Human discipline fails under pressure — automated enforcement does not. Rejected: guideline-based approach. | Platform Orchestrator |
| 2026-02-20 | Field worker and consumer apps are exempt from idle timeout | Field workers are actively using the app mid-route — forced logout would be catastrophic. Consumer sessions are handled by short JWT TTL, not idle warnings. Rejected: universal idle timeout (wrong for field use). | Platform Orchestrator + TrashTech Orchestrator |
| 2026-02-20 | Separate routes and layouts per audience — shared component library | Desktop and mobile-first users cannot share a layout. A responsive design that tries to serve both serves neither well. Rejected: single responsive layout (results in a poor desktop-on-mobile or mobile-on-desktop experience). | Platform Orchestrator |
| 2026-02-20 | Full-screen confirmation pages instead of modals on mobile | Modals are disorienting on small screens, especially when the user is in motion. A full-screen page is clear, tappable, and easy to cancel via back. Rejected: modal-on-mobile for confirmations (disorienting, small tap targets). | TrashTech Orchestrator |
| 2026-02-20 | Offline mutation queue must be idempotent | Non-idempotent queued mutations produce duplicates on retry. That is worse than requiring connectivity. Rejected: queuing all mutations regardless of idempotency. | Platform Orchestrator |

---

> See `docs/frontend/DOC-REVISION-STANDARDS.md` for the standards governing how this document is maintained.
