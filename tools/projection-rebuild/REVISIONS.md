# projection-rebuild — Revision History

## v1.0.0 — 2026-03-28

Initial proven release. Projection rebuild and blue-green swap orchestration tool.

- CLI with rebuild, status, verify, and list subcommands
- JWT-based RBAC authorization for all operations
- Deterministic digest computation for projection integrity verification
- Blue-green swap capability via shadow cursor tables
- All clippy warnings resolved; proof script passes
