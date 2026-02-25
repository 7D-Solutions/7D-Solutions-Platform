# control-plane — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.0.1 | 2026-02-25 | bd-1uce | Added graceful shutdown with SIGTERM/SIGINT signal handling. Server now drains in-flight requests before closing DB pool on shutdown. | Zero-downtime deploys require graceful shutdown to avoid dropping in-flight requests. | No |
| 1.0.0 | 2026-02-21 | bd-qvbg | Initial proof. All 23 tests passing (unit + integration against real DB). Handles tenant create, platform billing run, retention policy, AR client, tenant-registry client. Proof command: `./scripts/proof_control_plane.sh` | Module build complete. Phase 44 Track B promotion. | — |

## How to read this table

- **Version:** The version in the package file (`Cargo.toml`) after this change.
- **Date:** When the change was committed.
- **Bead:** The bead ID that tracked this work.
- **What Changed:** A concrete description of the change. Name specific endpoints, fields, events, or behaviors affected.
- **Why:** The reason the change was necessary.
- **Breaking?:** `No` if existing consumers are unaffected. `YES` if any consumer must change code to handle this version.

## Rules

- Add a new row for every version bump. One row per version.
- Do not edit old rows. If a previous change is reversed, add a new row explaining the reversal.
- The commit that bumps the version in the package file must also add the row here. Same commit.
- If the change is breaking (MAJOR version bump), the "Breaking?" column must describe what consumers need to change.
