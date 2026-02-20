# Frontend Document Revision Standards

> **Who this is for:** Claude Code agents. Written to be followed, not read.
> This is not documentation for human developers — it is instructions for agents maintaining vision and standards documents.

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | BrightHill | Created. Adopted as cross-app standard with TopazElk. |
| 1.1 | 2026-02-20 | BrightHill | Added Cross-Repo Path Convention section and full docs/ directory structure. Symlinks created: docs/apps/trashtech and docs/reference/fireproof. Eliminates absolute paths across all docs. |

---

## What Documents Live Here

```
docs/
  frontend/
    DOC-REVISION-STANDARDS.md          ← this file
    PLATFORM-FRONTEND-STANDARDS.md     ← shared rules for all platform apps
    TCP-UI-VISION.md                   ← Tenant Control Plane UI product vision
  apps/
    trashtech/                          ← symlink → /Users/james/Projects/TrashTech/docs/
    [next-app]/                         ← symlink → each app's docs/ folder
  reference/
    fireproof/                          ← symlink → /Users/james/Projects/Fireproof/frontend
```

Each app adds its own vision document here when its planning begins.

---

## Cross-Repo Path Convention

Never use absolute paths in documents. Use repo-relative paths via symlinks.

**Structure:**
- `docs/apps/<app-name>/` — symlink to each vertical app's `docs/` folder
- `docs/reference/<project-name>/` — symlink to reference projects (Fireproof, etc.)

**Examples:**
- TrashTech vision doc: `docs/apps/trashtech/VISION.md`
- Fireproof ESLint rules: `docs/reference/fireproof/eslint-local-rules/`
- Platform standards (from TrashTech repo): `docs/platform/frontend/PLATFORM-FRONTEND-STANDARDS.md`

**Adding a new app:** Create the symlink before writing any doc that references that app.
```bash
ln -s /Users/james/Projects/MyApp/docs docs/apps/myapp
```

**Why:** Absolute paths break when the machine changes or the repo moves. Repo-relative paths via symlinks work anywhere the symlinks are set up.

---

## Document Structure — Required in Every Vision and Standards Doc

Every document in `docs/frontend/` must have these sections in this order:

1. **Title + one-line description** (what the doc covers, who it's for)
2. **Revision History table** — immediately after the title
3. **Body** — the actual content
4. **Open Questions table** — unresolved decisions blocking bead creation
5. **Decision Log** — at the bottom
6. **Pointer to this file** — last line

Do not add sections, rename sections, or reorder sections without updating this standards doc.

---

## Revision History Table

**Location:** Immediately after the document title and scope block. Before any content.

**Format:**

```markdown
## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | YYYY-MM-DD | AgentName | Initial document — list sections created. |
| 1.1 | YYYY-MM-DD | AgentName | Added X section — reason why it was needed. |
| 1.2 | YYYY-MM-DD | AgentName | Updated Y — what changed and why. |
```

**Rules:**
- Every commit that changes the document content bumps the revision (1.0 → 1.1 → 1.2, etc.)
- Minor wording fixes that do not change meaning: no revision bump, just note in commit message
- Summary must say what changed AND why — not just what
- Changed By is the agent ID (`BrightHill`, `TopazElk`, etc.) or `User` when the human made the change directly
- If a decision is reversed, add a new revision row — do not edit old rows

---

## Decision Log Table

**Location:** Bottom of the document, above the pointer to this file.

**Format:**

```markdown
## Decision Log

Decisions that are settled. Do not re-open without an explicit user directive.

| Date | Decision | Rationale (includes what was NOT chosen) | Decided By |
|------|----------|------------------------------------------|-----------|
| YYYY-MM-DD | Short statement of what was decided | Why this was chosen. What was rejected and why. | Agent/User |
```

**Rules:**
- Every structural decision goes here when it is made — not after the fact
- Rationale MUST name what was NOT chosen. "Chose X" is not sufficient. "Chose X over Y because Z" is required.
- If a decision is reversed: add a new row, do not delete the old one. New row says: "Supersedes YYYY-MM-DD decision on [topic]. New decision: [what]. Reason for reversal: [why]."
- `Decided By` is the agent or human who made the call. If the user decided, write `User`. If agents proposed and user approved, write `User + AgentName`.

---

## Open Questions Table

**Location:** Before the Decision Log, after the main body content.

**Format:**

```markdown
## Open Questions

Do not create beads until all questions here are resolved.

| # | Question | Status |
|---|----------|--------|
| 1 | Question text | Open / ✅ Resolved — see Decision Log |
```

**Rules:**
- Every unresolved design question that blocks bead creation goes here
- When resolved: mark ✅ Resolved and add the decision to the Decision Log
- Do not delete resolved rows — mark them resolved so there is a record

---

## Commit Rules for Document Changes

Every commit touching a document in `docs/frontend/` must:

1. **Be a standalone commit** — do not bundle doc changes with code changes in the same commit
2. **Reference the active bead**: `[bd-xxx] docs: update TCP-UI-VISION.md — add landing page decision`
3. **State what changed in the commit message** — not just "update docs"
4. **Bump the Revision History** before committing

Example commit message:
```
[bd-xxx] docs: PLATFORM-FRONTEND-STANDARDS.md rev 1.4

- Added Decision Log section
- Added Revision History table
- Adopted cross-app doc standard from TopazElk
```

---

## What These Documents Are For

These documents are written so Claude Code agents can implement features correctly without asking the user to re-explain decisions.

**A document is doing its job when:**
- An agent can read it and know exactly what to build — no ambiguity
- An agent cannot accidentally undo a settled decision because the Decision Log explains the rationale
- An agent updating the document knows exactly where to add new content

**A document is failing when:**
- It uses aspirational language ("aim for", "try to", "ideally") instead of rules
- It omits why a decision was made (so the next agent re-litigates it)
- It mixes open questions with settled decisions
- It is written for a human reader rather than an agent following instructions

**Write rules as facts:** `Touch targets are 48×48px CSS minimum.` Not: `Aim for accessible touch targets.`

**Write prohibitions explicitly:** `Never use raw <button> elements. Import Button from components/ui/.`

**Name the exact file path:** `Store the Zustand store in infrastructure/state/tabStore.ts.` Not: `Put it in the state folder.`

---

> See `docs/frontend/PLATFORM-FRONTEND-STANDARDS.md` for shared standards across all platform apps.
