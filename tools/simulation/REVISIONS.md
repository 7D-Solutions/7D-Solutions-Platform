# simulation — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.0.0 | 2026-03-28 | bd-33602 | Initial stable release. Deterministic seed model with ChaCha20 RNG, failure injection engine with payment/webhook/event failure types, concurrent scheduler with barrier synchronization. 10 tests prove determinism and correctness. | Promotion to proven status after passing proof script. | N/A |
