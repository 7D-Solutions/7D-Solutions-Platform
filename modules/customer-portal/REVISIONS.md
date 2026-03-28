# customer-portal — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.0.0 | 2026-03-28 | bd-1xe02 | Initial promotion. External customer auth boundary with RS256 JWT, Argon2 password hashing, refresh token rotation, tenant-isolated portal users, document visibility via doc-mgmt distribution check, status feed with acknowledgments, outbox event emission for all auth lifecycle events. Proof script, clippy clean, 5 tests pass (1 unit + 4 real-DB integration). Security audit: no blocking findings. | Production readiness gate for first paying customer (Fireproof ERP). | No |
