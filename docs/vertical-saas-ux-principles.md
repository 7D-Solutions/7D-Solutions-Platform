# Vertical SaaS UX — The Goal

You are building a vertical SaaS app. Here's what "intuitive" means for the people who use it every day. Implementation is yours to decide — these are the ideas that should shape your choices.

## The core idea

Design around the thing the user actually works on. In an ERP it's the job. In a routing app it's the route or the pickup. In a legal app it's the matter. Whatever the user would describe as *"the thing I work on all day"* — that's the object. Every screen that matters radiates from it.

When the user clicks that object, everything they need to act on it is on one screen: current status, the next step, related data, history. They shouldn't have to navigate to other modules or pages to do their job.

If the central object for this vertical isn't obvious from the codebase or docs, flag and ask the product owner before designing.

## What this looks like in practice

**Always show the next step.** The user shouldn't have to figure out what to do next. A prominent action button. If they can't act, tell them why in plain English — not a greyed-out button with no explanation.

**Show each user only what they need.** Different roles looking at the same object need different views — a shop floor operator doesn't need cost data, an admin doesn't need the tap-complete button. If the role boundaries for this vertical aren't clear from the codebase, flag and ask the product owner before designing.

**Pre-fill from context.** If the system already knows the job number, the customer, the current step — populate it. Don't make the user re-enter what the system has.

**Go object-to-object, not module-to-module.** Clicking a related record (customer, work order, invoice) opens it in context. The user can always see where they came from and get back.

**Work on a tablet.** Shop floors, trucks, and field sites don't have desktops. If it only works on a laptop, it's not done.

**Never a dead end.** Every screen — including empty states, user errors, and system failures (API timeout, save failed) — shows the user what they can do next. No blank screens. No "an error occurred" with nowhere to go.

## What kills adoption

Real users consistently cite these across every vertical — manufacturing, field service, legal, healthcare, logistics. Avoid them:

1. Too many clicks to do a common task
2. Cluttered screens with irrelevant fields
3. No clear next step — user has to figure out what to do
4. Slow or laggy, especially on mobile
5. Steep learning curve — if users can't learn by doing, adoption fails
6. Module-hopping — forcing navigation to complete a logical step
7. Generic interfaces — feels built for everyone and no one

## Sanity check before shipping

- Can a new user figure out the next step without being told?
- Does the screen show anything this role doesn't need?
- Is the most common action right there, not buried?
- Does it work on a tablet — no hover dependencies, touch targets large enough, layout doesn't break?
- If the user is blocked, does the UI tell them why and how to unblock?
- If the user taps or clicks wrong, is the mistake recoverable?

If any answer is no, it's not ready.

## When something conflicts

If a feature request asks you to break one of these ideas — too many clicks, hidden next step, module-hop to complete a flow — flag it, explain what it breaks, and ask the product owner before proceeding.
