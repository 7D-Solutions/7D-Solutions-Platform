# event-bus — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.0.1 | 2026-03-28 | bd-1oqbu | Builder methods (`with_source_version`, `with_schema_version`, `with_actor`) now accept `impl Into<String>` instead of `String`. Eliminates forced allocations when callers pass `&str` literals. | Performance optimization from extreme-software-optimization analysis. | No |
| 1.0.0 | 2026-03-28 | bd-2k6o9 | Initial proof. EventBus trait (publish/subscribe), NatsBus (NATS JetStream), InMemoryBus (test/dev), EventEnvelope with constitutional metadata (tenant isolation, mutation class, actor identity, tracing context, merchant context money-mixing guard), TracingContext propagation, outbox validation gate, consumer retry with exponential backoff, connect_nats URL auth helper. 57 unit + 14 integration + 11 doc-tests passing. Clippy zero warnings. Downstream crates compile clean. | Module proven. Every event in the platform flows through this crate. | — |
