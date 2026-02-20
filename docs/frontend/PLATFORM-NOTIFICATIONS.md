# Platform Frontend — Notification System

> **Who reads this:** Any agent implementing any alert, status message, confirmation, or event notification.
> **What it covers:** The complete rules for how notifications work — what channel to use, when, and how. Browser notifications are prohibited. Two platform channels exist: toast and notification center.
> **Rule:** Every notification goes through one of the two platform channels. There is no third option.

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | Platform Orchestrator | Extracted from PLATFORM-FRONTEND-STANDARDS.md rev 1.8. Browser notification prohibition, toast channel rules, notification center pattern (bell + badge + panel), toast vs modal threshold, foundation requirements. Decision Log populated from master. |

---

## The Absolute Rule: No Browser Notifications

**`Notification.requestPermission()` and `new Notification(...)` are prohibited on all platform apps. No exceptions.**

Browser notifications hand control of the user experience to the operating system. They appear outside the app. They are styled by the OS. Their timing and display are outside the platform's control. They require user permission, which many users deny, making them unreliable by design.

**ESLint rule `no-browser-notifications` blocks these calls. Violations fail the build.**

The platform controls the notification experience — not the browser.

---

## Two Channels

Every notification on this platform goes through exactly one of two channels:

| Channel | Type | When it disappears | When to use |
|---------|------|-------------------|------------|
| **Toast** | Transient | 4 seconds, auto-dismiss | Action succeeded, process finished, non-critical status change |
| **Notification Center** | Persistent | When staff dismisses it | Anything important enough that a missed toast would be a problem |

**Why both:** A 4-second toast can be missed if the user is looking at a different part of the screen. The notification center provides a persistent history — staff can check the bell at any time and see everything they may have missed.

---

## Channel 1: Toast

### Rules

- Duration: **4 seconds**. Auto-dismiss. No action required.
- Maximum **one toast visible at a time.** If a second fires before the first dismisses, it queues — it does not stack.
- Toast is **read-only** — it never contains form inputs or action buttons (use a modal for that).
- Toast positions at top-right of the screen. Never obstructs navigation or primary content.

### When to Use Toast

| Use toast for | Never use toast for |
|--------------|---------------------|
| Action succeeded (save, submit, status change) | Destructive or irreversible actions (use modal) |
| Background process finished (export ready, sync complete) | Actions requiring user input or reason capture (use modal) |
| Non-critical informational events | Anything affecting billing (use modal) |
| Connectivity restored | Critical system alerts (use notification center) |

### Toast Severity Levels

| Severity | Color | When |
|----------|-------|------|
| `success` | Green | Operation completed successfully |
| `error` | Red | Operation failed — include what to do next |
| `warning` | Amber | Completed but with caveats |
| `info` | Blue | Neutral information |

---

## Channel 2: Notification Center

### What It Is

A bell icon in the top bar with a numeric count badge. Platform alerts accumulate here and remain until the staff member explicitly dismisses them. Zero count hides the badge.

### Standard Pattern

**Layout:**
- Bell icon in the top navigation bar, right-aligned near the user menu
- Numeric badge on the bell (hidden at zero, max display "99+")
- Clicking the bell opens a dropdown panel — not a full page, not a modal

**Panel contents:**
- "Clear all" button at the top
- Notifications ordered newest-first
- Each notification row:
  - Severity icon (info/warning/error color-coded)
  - Title (short, plain English)
  - Description (one sentence — what happened)
  - Timestamp (always date + time — see `PLATFORM-LANGUAGE.md` → Formatter Standards)
  - Dismiss button (X) on each row
- Unread notifications visually distinct from read (bold title, colored left border)
- Clicking a notification may navigate to the relevant record (optional, per event type)

**State:**
- `notificationStore` — Zustand store, **in-memory only** (not localStorage)
- On page refresh, notification center clears
- Persistence to backend API is a deferred decision — see Deferred Decisions in `PLATFORM-FRONTEND-STANDARDS.md`

### Severity Levels

| Severity | Icon color | When |
|----------|-----------|------|
| `info` | Blue | Informational events (billing run complete, export ready) |
| `warning` | Amber | Events requiring attention soon (tenant approaching limit) |
| `error` | Red | Events requiring immediate attention (service down, payment failure) |

### What Belongs in the Notification Center

Each app's vision document defines which system events generate a notification. The rule: **only create a notification if a staff member would want to know and act on it.** Notification fatigue reduces trust in the system — if the bell always has a count, staff stop checking it.

**Examples for TCP UI:** tenant goes past due, backend service health degrades, billing run completes with errors.
**Not a notification:** routine billing run success (use toast), tenant detail viewed (no notification).

### Foundation Requirements (Staff Apps)

Staff-facing apps add to their Foundation bead:
- [ ] `infrastructure/state/notificationStore.ts` — in-memory Zustand store
- [ ] `components/ui/NotificationCenter.tsx` — bell icon + badge + dropdown panel
- [ ] `components/ui/NotificationItem.tsx` — individual notification row with dismiss

---

## Toast vs Modal Threshold

For every alert or confirmation, the question is: toast, modal, or notification center? This table is the decision rule.

### Use a Toast When

- An action **succeeded** and requires no further input
- A **background process finished** (export ready, sync complete)
- The event is **non-critical** and losing it would not cause a problem

### Use a Modal When

- The action is **destructive or irreversible** (delete, terminate, cancel)
- The action requires **a reason or confirmation input** from the user
- The action **affects billing** (any invoice or payment action)
- Confirmation language names what will happen specifically: "Terminate Acme Corp and cancel all subscriptions?" — not "Are you sure?"

### Use the Notification Center When

- The event is important enough that a **missed toast would be a problem**
- The event happened **while the user was looking at a different screen**
- The event has **no immediate action required** but staff should know about it

### Field Worker Apps (Mobile)

No multi-step modal flows. If a confirmation is needed, show a **full-screen confirmation page** — not a modal layered over the work screen. Modals on mobile are disorienting when the user is in motion.

---

## Open Questions

Do not create beads until these are resolved.

| # | Question | Status |
|---|----------|--------|
| — | No open questions at this time. | — |

---

## Decision Log

Decisions specific to notifications. Do not re-open without an explicit user directive.

| Date | Decision | Rationale (includes what was NOT chosen) | Decided By |
|------|----------|------------------------------------------|-----------|
| 2026-02-20 | Browser notifications (Notification API) are prohibited on all platform apps | Browser notifications hand UX control to the OS. They appear outside the app, are styled by the OS, require user permission (often denied), and are outside platform control. Rejected: `Notification.requestPermission()` / OS push notifications entirely. | User |
| 2026-02-20 | Two notification channels: toast (transient) + notification center (persistent) | Toast alone: missed events are gone in 4 seconds. Notification center alone: no immediate feedback. Both together provide immediacy and history. Rejected: toast only (missed events lost), notification center only (poor immediate feedback). | User |
| 2026-02-20 | Notification center state is in-memory only — not persisted to localStorage | localStorage persistence would require clearing logic, expiry logic, and cross-tab sync. In-memory is simpler and sufficient. If persistence is needed, it goes to the backend API. Rejected: localStorage (complex without meaningful benefit). | Platform Orchestrator |
| 2026-02-20 | Global popup manager (third notification channel) rejected | Toast = transient, modal = blocking. Two channels covers every case. A third channel creates decision overhead: "is this a toast, modal, or popup?" Rejected: queued popup system separate from toast and modal. | TrashTech Orchestrator + Platform Orchestrator |
| 2026-02-20 | Modal confirmation language must name what will happen specifically | "Are you sure?" is ambiguous. Staff must know exactly what they are confirming. Rejected: generic confirmation language ("are you sure?", "confirm action?"). | Platform Orchestrator |

---

> See `docs/frontend/DOC-REVISION-STANDARDS.md` for the standards governing how this document is maintained.
