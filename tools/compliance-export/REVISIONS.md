# compliance-export — Revision History

## v1.0.0 — 2026-03-28

Initial proven release. Compliance evidence pack generation for audit and regulatory export.

- Export audit logs and ledger data (AR, Payments, GL) per tenant
- Evidence pack generation for closed accounting periods with tamper-evident hash chain
- SHA-256 checksums for all exported data files
- JSONL and CSV export formats with deterministic ordering
- Unit tests for checksum computation, serialization roundtrips, and file output
- CLI with export and evidence-pack subcommands
