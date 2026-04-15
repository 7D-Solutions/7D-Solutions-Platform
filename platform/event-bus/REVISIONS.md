# event-bus — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.


## 2.1.1
- chore: rustfmt reflow + regenerate typed clients (no behavior change)

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.1.0 | 2026-04-13 | bd-ejygy | Add `stream_config.rs` with per-stream dedup window policy (financial/operational/notification classes). Streams declare class via manifest; startup applies dedup window on stream init. E2E test validates dedup behavior. | NATS JetStream dedup window was a single global default; financial streams need longer windows than notifications to match replay semantics. | No |
| 2.0.0 | 2026-04-01 | bd-p3xps | Added `async fn health_check(&self) -> bool` to `EventBus` trait. NatsBus checks NATS connection state; InMemoryBus always returns true. Updated NATS tests to use `connect_nats()` with `NATS_URL` env var. | SDK PnP 6: health auto-probing requires trait-level health check. | Yes — all `impl EventBus` blocks must add `health_check()`. Migration: add `async fn health_check(&self) -> bool { true }` to any custom implementor. |
| 1.0.1 | 2026-03-28 | bd-1oqbu | Builder methods (`with_source_version`, `with_schema_version`, `with_actor`) now accept `impl Into<String>` instead of `String`. Eliminates forced allocations when callers pass `&str` literals. | Performance optimization from extreme-software-optimization analysis. | No |
| 1.0.0 | 2026-03-28 | bd-2k6o9 | Initial proof. EventBus trait (publish/subscribe), NatsBus (NATS JetStream), InMemoryBus (test/dev), EventEnvelope with constitutional metadata (tenant isolation, mutation class, actor identity, tracing context, merchant context money-mixing guard), TracingContext propagation, outbox validation gate, consumer retry with exponential backoff, connect_nats URL auth helper. 57 unit + 14 integration + 11 doc-tests passing. Clippy zero warnings. Downstream crates compile clean. | Module proven. Every event in the platform flows through this crate. | — |