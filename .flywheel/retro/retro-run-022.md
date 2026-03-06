# Retro Run #022 — 2026-03-06

**Trigger:** count-based (5 closes since last retro)
**Analysis window:** 5 closes since retro 021 (retro_seq 467–471)
**Runner:** MaroonHarbor (manual — run-retro.sh not found, bd-rz7qv)

## Beads Analyzed

| Bead | Title | Agent | Commits | Key pattern |
|------|-------|-------|---------|-------------|
| bd-13yjd | Phase C2: In-process + final inspection record types linked to WO/operations | CopperRiver | 2 | Production-linked inspection subtypes |
| bd-nl0zm | Phase B: Explicit component issue workflow (Production to Inventory via events) | PurpleCliff | 1 | Cross-module event consumer pair |
| bd-23t54 | Fix gateway health check IPv6 resolution failure | BrightHill | 1 | Docker Alpine IPv6 trap |
| bd-1r5sw | Restart Docker and bring up services | BrightHill | 0 | Ops (no code) |
| bd-1j2hj | Phase B: FG receipt at rolled-up cost (Production to Inventory) + arithmetic spot-check | PurpleCliff | 2 | Cross-module consumer with derived cost |

## Signals

- **Closes in window:** 5
- **Avg commits per bead:** 1.2 (excluding ops bead: 1.5)
- **Agent spread:** PurpleCliff (2), BrightHill (2), CopperRiver (1)
- **Zero-commit beads:** 1 (ops restart)
- **Child beads spawned:** 0
- **Cross-module event consumer beads:** 2 (component issue + FG receipt)

## Patterns Observed

### 1. Cross-module consumers separate testable logic from NATS loop
Both component_issue_consumer and fg_receipt_consumer expose a pure `process_*` function that takes `(pool, event_id, payload, correlation_id, causation_id)`. The NATS consumer loop is a thin wrapper that deserializes the envelope and delegates. This allows integration tests to exercise the full processing path against real Postgres without needing NATS. This pattern appeared independently in both beads.

### 2. Consumers define local payload types mirroring the producer — no cross-crate dependency
Both consumers define their own `*Payload` structs with `#[derive(Deserialize)]` rather than importing from the producing crate. This keeps module boundaries clean and prevents compile-time coupling between producer and consumer crates. The contract is the JSON envelope schema, not a Rust type.

### 3. Deterministic idempotency keys for multi-item events use event_id + index
Component issue consumer builds keys as `format!("ci-{}-{}", event_id, idx)` for each item in a multi-item request. FG receipt uses `format!("fg-receipt-{}", event_id)`. Both patterns produce deterministic, reproducible keys from the incoming event, ensuring safe replay without deduplication state outside the database.

### 4. Cost-derived receipts guard against zero or missing cost data
The FG receipt consumer queries component issue costs before computing the rolled-up unit cost. If no component issues exist for the work order, it rejects the receipt with a clear error rather than creating a phantom FG entry at zero cost. This prevents silent data corruption in cost accounting.

### 5. Docker Alpine health checks must use 127.0.0.1, not localhost
Alpine's wget resolves 'localhost' to IPv6 (::1) first. If the service only binds IPv4 (0.0.0.0), health checks fail silently and the container reports unhealthy for hours. The fix is one character: use 127.0.0.1 explicitly. This affected the gateway for 6+ hours before diagnosis.

### 6. Cross-module event pairs follow a command-request pattern
Production enqueues a `.requested` event (e.g., `production.component_issue.requested`), and Inventory's consumer processes it and produces the actual stock mutation. The producing module never directly calls the consuming module's API. This pattern decouples the modules while maintaining transactional integrity within each side.

### 7. Inspection subtypes extend a single table with nullable production columns
In-process and final inspections reuse the inspections table with added nullable columns (wo_id, op_instance_id) plus an inspection_type discriminator. In-process requires both wo_id and op_instance_id; final requires wo_id and optional lot_id. This avoids table proliferation while supporting type-specific queries.
