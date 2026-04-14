# Event Deduplication Windows

## Problem

NATS JetStream deduplicates messages within a configurable window using the
`Nats-Msg-Id` header. The **default window is 2 minutes**. When a consumer lags
more than 2 minutes and the publisher retries, JetStream forgets the original
message ID — "exactly-once" silently degrades to "at-least-once".

For financial events (AP bills, AR invoices, GL journal entries, payments), a
duplicate delivery means a **double-posted ledger entry** — a compliance
violation. The 2-minute default is not acceptable.

## Solution

Each platform JetStream stream is configured with an explicit `duplicate_window`
matched to the risk profile of the events it carries. The configuration lives in
`platform/event-bus/src/stream_config.rs` and is applied idempotently at startup
by every module that connects to NATS.

## Stream Classes and Windows

| Class        | Window | Streams              | Modules                                                    |
|--------------|--------|----------------------|------------------------------------------------------------|
| Financial    | 24h    | `FINANCIAL_EVENTS`   | ap, ar, gl, payments, billing, treasury, subscriptions, ttp |
| Operational  | 1h     | `OPERATIONAL_EVENTS` | production, inventory, shipping, maintenance, workflow, quality, fixed-assets, timekeeping, workforce, numbering |
| Notification | 1h     | `NOTIFICATION_EVENTS`| notifications                                              |
| System       | 24h    | `SYSTEM_EVENTS`      | tenant-registry (tenant lifecycle events)                  |

> **Note on `auth.*`:** The `AUTH_EVENTS` stream is managed by the `identity-auth`
> module separately. It covers `auth.>` and should also have a 24h dedup window
> configured in `platform/identity-auth/src/jetstream_setup.rs`.

## Subject Routing

Each stream captures all subjects under its prefixes:

### FINANCIAL_EVENTS
Subjects: `ap.>`, `ar.>`, `gl.>`, `payments.>`, `billing.>`, `treasury.>`,
`subscriptions.>`, `ttp.>`

### OPERATIONAL_EVENTS
Subjects: `production.>`, `inventory.>`, `shipping.>`, `maintenance.>`,
`workflow.>`, `quality.>`, `fixed-assets.>`, `timekeeping.>`, `workforce.>`,
`numbering.>`

### NOTIFICATION_EVENTS
Subjects: `notifications.>`

### SYSTEM_EVENTS
Subjects: `tenant.>`

## Why 24h for Financial?

A 24h window means JetStream remembers every `Nats-Msg-Id` published in the last
24 hours. If a consumer is down for up to 24 hours and the publisher retries the
same event (same `event_id`), NATS deduplicates at the broker — the consumer
sees it only once.

Beyond 24 hours, the consumer's own idempotency layer (PostgreSQL `event_dedup`
table in `platform-sdk`) is the last line of defence. The combination provides
overlapping protection:

1. **NATS JetStream dedup** — fast path, broker-level, covers the first 24h
2. **DB idempotency** (`with_dedupe` in `event-consumer`) — permanent, survives restarts

## Publisher Requirements

For NATS dedup to work, publishers **must** set the `Nats-Msg-Id` header to the
`EventEnvelope.event_id` on every JetStream publish:

```rust
use async_nats::jetstream::{self, context::Publish};

js.send_publish(
    subject,
    Publish::build()
        .payload(serialized_envelope.into())
        .message_id(envelope.event_id.to_string()),
)
.await?
.await?;
```

The platform outbox publisher (`platform/platform-sdk/src/publisher.rs`) sets
this header automatically for all outbox-relayed events.

## Startup Behaviour

When a module starts with `bus.type = "nats"`, `phase_a` in `startup.rs` calls
`event_bus::ensure_platform_streams(nats_client)` after connecting. This function:

1. For each stream in `all_stream_definitions()`:
   - If the stream **does not exist** → creates it with the configured dedup window
   - If the stream **already exists** → updates its config (idempotent, preserves messages)
2. Logs each stream at `INFO` level so operators can confirm the config was applied

Stream setup failure is **non-fatal**: the module starts and falls back to the
2-minute NATS default. This avoids blocking all modules when JetStream is
temporarily unavailable, but operators will see a `WARN` log.

## Verification

```bash
# Run unit tests (no NATS required)
./scripts/cargo-slot.sh test -p event-bus dedup_window -- --nocapture

# Run all dedup tests including real NATS integration
NATS_URL=nats://platform:dev-nats-token@localhost:4222 \
  ./scripts/cargo-slot.sh test -p event-bus dedup_window -- --nocapture

# Inspect a stream's config after startup
nats stream info FINANCIAL_EVENTS
# Look for: Duplicate Window: 24h0m0s
```

## Adding New Streams

1. Add a new `StreamDefinition` to `all_stream_definitions()` in
   `platform/event-bus/src/stream_config.rs`
2. Assign the appropriate `StreamClass` (or add a new class if needed)
3. Add `stream_class = "..."` to the module's `[bus]` section in `module.toml`
4. Update this document

Do not create streams directly in module startup code — all stream definitions
must go through `all_stream_definitions()` to stay in sync.
