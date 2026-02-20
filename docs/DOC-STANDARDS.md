# Platform Documentation Standards

> **Who this is for:** Claude Code agents. Written to be followed, not read.
> **What it covers:** How every document in this platform repo is structured, maintained, and governed — frontend standards, consumer guide, vision docs, and any future doc family.
> This is not documentation for human developers — it is instructions for agents.

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | Platform Orchestrator | Created. Unified standard consolidating docs/frontend/DOC-REVISION-STANDARDS.md and docs/CG-DOC-STANDARDS.md. Both reduced to pointers to this file. |

---

## Document Families

The platform has two families of documentation, each with its own directory and conventions. Both share the base rules in this document.

| Family | Location | Index file | Standards extension |
|--------|----------|------------|---------------------|
| Consumer Guide | `docs/CG-*.md` | `docs/PLATFORM-CONSUMER-GUIDE.md` | [CG-specific rules](#consumer-guide-specific-rules) |
| Frontend Standards | `docs/frontend/*.md` | `docs/frontend/PLATFORM-FRONTEND-STANDARDS.md` | [Frontend-specific rules](#frontend-specific-rules) |

---

## Base Rules — Apply to ALL Documents

### Required Sections (in order)

Every document in either family must have these sections in this order:

1. **Title** — `# [Document Name]`
2. **Header block** — who reads it, what it covers, link to the index file for this family
3. **Revision History table** — immediately after the header block, before any content
4. **Body** — the actual content
5. **Footer pointer** — last line links back to the family index

Topic files additionally require a **Contents section** (TOC) between the header block and the Revision History, listing every `##` section with anchor links.

### Header Block Format

```markdown
> **Who reads this:** [audience]
> **What it covers:** [one-line scope summary]
> **Parent:** [link to the index file for this family]
```

### Revision History Table

**Location:** Immediately after the header block. Before any content.

```markdown
## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | YYYY-MM-DD | Platform Orchestrator | Initial document — list sections created. |
| 1.1 | YYYY-MM-DD | Platform Orchestrator | Added X section — reason why it was needed. |
```

**Rules:**
- Every commit that adds, removes, or substantively changes content bumps the revision (1.0 → 1.1 → 1.2)
- Minor wording fixes that don't change meaning: no revision bump, note in commit message only
- Summary must say what changed AND why — not just what
- **Changed By uses roles, not agent names.** Agent names change every session. Use stable roles: `User`, `Platform Orchestrator`, `TrashTech Orchestrator`. Never a session agent name like `BrightHill` or `TopazElk`.
- Do not edit old rows — add new rows for changes and reversals

### Footer Pointer

Last line of every topic file:
```markdown
> See [index file name](./path) for [brief description of what the index covers].
```

### File Size Limit

Keep every doc file under 450 lines. If adding a section would exceed this, split the section into a new topic file and add a Quick Find entry pointing to it.

---

## Commit Rules for Document Changes

Every commit touching a documentation file must:

1. **Be a standalone commit** — do not bundle doc changes with code changes in the same commit
2. **Reference the active bead**: `[bd-xxx] docs: CG-AUTH.md rev 1.1 — added service token section`
3. **State what changed and why** in the commit message body
4. **Bump the Revision History** in every file touched before committing

Example:
```
[bd-xxx] docs: PLATFORM-CONSUMER-GUIDE.md + CG-TENANCY.md rev 1.1

- CG-TENANCY.md: added support sessions technical mechanism section
- PLATFORM-CONSUMER-GUIDE.md: added Quick Find rows for support session content
Reason: TopazElk needed the BFF route pattern before implementing SupportSessionBanner
```

---

## Governance — Who Can Change What

**Platform Orchestrator** owns all documents in `docs/`. No other agent edits platform documents without authorization.

**App teams** (TrashTech Orchestrator, future app teams) read platform documents and implement what they say. They do not maintain parallel versions of platform rules in their own repos.

### Change Request Process

When any agent or app team needs a platform document changed, they send mail to the platform orchestrator with:
- Subject: `Doc change request — [file name] — [short description]`
- Current content (exact quote)
- Proposed content
- Reason the current content doesn't work
- Which beads are blocked without the change

The platform orchestrator approves or rejects. If approved, the orchestrator commits the change. The requesting team implements after the commit is confirmed.

**Changed By uses roles, not agent names** — same rule as the Revision History table.

### What Requires a Change Request

- Adding, removing, or renaming sections in any platform doc
- Changing any rule or standard
- Registering a new app or adding a new topic file

### What Does NOT Require a Change Request

- Platform Orchestrator updating any platform doc
- App teams updating their own vision documents in their own repos

---

## What Good Documentation Looks Like

These documents exist so agents can implement correctly without asking the user to re-explain decisions.

**A document is doing its job when:**
- An agent can read it and know exactly what to build — no ambiguity
- An agent cannot accidentally undo a settled decision because the rationale is recorded
- An agent updating the document knows exactly where to add new content

**A document is failing when:**
- It uses aspirational language ("aim for", "try to", "ideally") instead of rules
- It omits why a decision was made (the next agent re-litigates it)
- It presents planned features as implemented facts
- It is written for a human reader rather than an agent following instructions

**Write rules as facts:** `Touch targets are 48×48px CSS minimum.` Not: `Aim for accessible touch targets.`

**Write prohibitions explicitly:** `Never use raw <button> elements. Import Button from components/ui/.`

**Name the exact file path:** `Store the Zustand store in infrastructure/state/tabStore.ts.` Not: `Put it in the state folder.`

---

## Consumer Guide — Specific Rules

*These rules apply only to `docs/CG-*.md` and `docs/PLATFORM-CONSUMER-GUIDE.md`.*

### What Files Belong Here

```
docs/
  PLATFORM-CONSUMER-GUIDE.md    ← master index — first file any agent opens
  CG-DOC-STANDARDS.md           ← pointer to this file (DOC-STANDARDS.md)
  CG-AUTH.md                    ← HTTP headers, error format, identity-auth, JWT verification
  CG-EVENTS.md                  ← NATS, EventEnvelope, outbox pattern, Integrations module
  CG-MODULE-APIS.md             ← Party Master, AR module, First Invoice flow
  CG-REFERENCE.md               ← Env vars, Cargo.toml deps, local dev, E2E tests, source index
  CG-TENANCY.md                 ← Tenant provisioning, DB-per-tenant, per-app roles, support sessions
```

New topic files use the naming pattern `CG-[TOPIC].md` where TOPIC is a short uppercase noun.

### Source Verification Requirement

Every API fact must include a source reference:
```
Source: path/to/file.rs → StructOrFunctionName
```

**What counts as an API fact requiring a source:** HTTP endpoints (method, path, request/response shape), struct field names and types, enum variants, error codes, behavioral guarantees.

**What does not need a source:** Architectural explanations, patterns, rules derived from multiple sources.

**Planned but not implemented:** Mark as `[PLANNED — not yet in source]`. Never present a planned API as implemented fact.

**Last verified line:** The master index header includes `Last verified: YYYY-MM-DD against commit <sha>`. Update this when re-verifying against a new commit.

### Quick Find Table — Maintenance

The Quick Find table in `PLATFORM-CONSUMER-GUIDE.md` maps tasks to exact file + section. It is the primary navigation tool.

**Add a row when** a new section is added to any topic file.
**Remove a row when** a section is deleted.
**Update anchor links** when a section is renamed.

Row format:
```markdown
| I need to… [verb phrase] | [CG-FILE.md → Section Name](./CG-FILE.md#anchor) |
```

Task descriptions start with a verb ("Create an AR customer", "Set required environment variables"). Not topic names ("AR customers", "Env vars").

### Consumer Guide Does Not Use Decision Logs

Consumer Guide files are API reference, not vision or design docs. They do not have Open Questions or Decision Log sections — those belong in vision docs (see Frontend section below).

### Adding a New Consumer Guide Topic File

1. Create `docs/CG-[TOPIC].md` with the required structure
2. Add a row to the Topic Files table in `PLATFORM-CONSUMER-GUIDE.md`
3. Add Quick Find rows for every section in the new file
4. Add the new file to "What Files Belong Here" in this standards doc
5. Commit the new file and all index updates in the same commit

---

## Frontend Standards — Specific Rules

*These rules apply only to `docs/frontend/*.md`.*

### What Files Belong Here

```
docs/frontend/
  DOC-REVISION-STANDARDS.md          ← pointer to this file (DOC-STANDARDS.md)
  PLATFORM-FRONTEND-STANDARDS.md     ← INDEX — start here, then go to the topic file you need
  PLATFORM-COMPONENTS.md             ← CSS tokens, Button, StatusBadge, Modal, DataTable, forms
  PLATFORM-STATE.md                  ← Zustand stores, standard hooks, ESLint enforcement
  PLATFORM-LANGUAGE.md               ← Language rules, formatter standards
  PLATFORM-NOTIFICATIONS.md          ← Toast, notification center, browser notification rules
  PLATFORM-MOBILE.md                 ← Mobile constraints, offline pattern, multi-audience apps
  PLATFORM-FOUNDATION.md             ← Foundation bead checklist, Infrastructure Map, testing
  TCP-UI-VISION.md                   ← Tenant Control Plane UI product vision (Phase 41)
  [app]-VISION.md                    ← Each app adds its vision doc here when planning begins
```

### Open Questions Table

Frontend documents (vision docs and standards docs) include an Open Questions section. Consumer Guide files do not.

**Location:** Before the Decision Log, after the main body content.

```markdown
## Open Questions

Do not create beads until all questions here are resolved.

| # | Question | Status |
|---|----------|--------|
| 1 | Question text | Open |
| 2 | Question text | ✅ Resolved — see Decision Log |
```

Do not delete resolved rows — mark them resolved so there is a record.

### Decision Log Table

Frontend documents record architectural decisions so agents cannot re-litigate them.

**Location:** Bottom of the document, above the footer pointer.

```markdown
## Decision Log

Decisions that are settled. Do not re-open without an explicit user directive.

| Date | Decision | Rationale (includes what was NOT chosen) | Decided By |
|------|----------|------------------------------------------|-----------|
| YYYY-MM-DD | Short statement of what was decided | Why this was chosen. What was rejected and why. | User / Platform Orchestrator |
```

**Rules:**
- Every structural decision goes here when it is made
- Rationale MUST name what was NOT chosen. "Chose X" is not sufficient. "Chose X over Y because Z" is required.
- If a decision is reversed: add a new row, do not delete the old one
- **Decided By uses roles, not agent names** — same rule as Changed By

### Cross-Repo Path Convention

Never use absolute paths in documents. Use repo-relative paths via symlinks.

```
docs/apps/<app-name>/     ← symlink to each vertical app's docs/ folder
docs/reference/<project>/ ← symlink to reference projects (Fireproof, etc.)
```

Adding a new app: `ln -s /Users/james/Projects/MyApp/docs docs/apps/myapp`

### Vision Docs vs Standards Docs

- **Vision docs** (`TCP-UI-VISION.md`, `[app]-VISION.md`): product intent, screen descriptions, scope decisions, open questions, decision log
- **Standards docs** (`PLATFORM-COMPONENTS.md`, `PLATFORM-STATE.md`, etc.): rules, patterns, component specs — no open questions, decision log records final decisions only

---

> This is the single source of truth for all platform documentation standards.
> `docs/frontend/DOC-REVISION-STANDARDS.md` and `docs/CG-DOC-STANDARDS.md` are pointers to this file.
