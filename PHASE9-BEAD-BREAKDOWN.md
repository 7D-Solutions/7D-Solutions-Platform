# Phase 9: GL Module MVP - Bead Breakdown

**Source:** ChatGPT Strategic Planning Session
**Date:** 2026-02-12
**Conversation:** https://chatgpt.com/g/g-p-698c7e2090308191ba6e6eac93e3cc59/c/698e2d92-bbd0-8326-bd55-9ae4cfdbe3a3

## Overview

9-bead breakdown for GL Module MVP implementation. Agent-executable, parallel-friendly, maps cleanly to Phase 9 constraints.

## Bead List

1. **p9a-01** — Scaffold GL Module Skeleton (P0, no deps)
2. **p9a-02** — Add GL Postgres + Compose Wiring (P0, deps: p9a-01)
3. **p9a-03** — Migrations: Journal + Standard Event Tables (P0, deps: p9a-02)
4. **p9a-04** — DB Access Layer (sqlx) + Transaction Helpers (P0, deps: p9a-03)
5. **p9a-05** — Contract Types + Envelope Validation (P0, deps: p9a-01)
6. **p9a-06** — NATS Consumer: posting.requested → Persist Journal (P0, deps: p9a-04, p9a-05)
7. **p9a-07** — Dockerfile + Module Build Integration (P0, deps: p9a-01, p9a-02)
8. **p9a-08** — Extend Real Docker E2E: GL Assertions + Idempotency (P0, deps: p9a-06, p9a-02, p9a-03)
9. **p9a-09** — DLQ Behavior Test + Observability Touches (P1, deps: p9a-06)

## Parallel Execution Opportunities

**Wave 1 (No dependencies):**
- p9a-01: Scaffold GL Module Skeleton

**Wave 2 (Depends on p9a-01):**
- p9a-02: Add GL Postgres + Compose Wiring
- p9a-05: Contract Types + Envelope Validation
- p9a-07: Dockerfile + Module Build Integration (also depends on p9a-02)

**Wave 3 (Depends on Wave 2):**
- p9a-03: Migrations (depends on p9a-02)

**Wave 4 (Depends on Wave 3):**
- p9a-04: DB Access Layer (depends on p9a-03)

**Wave 5 (Depends on Wave 4):**
- p9a-06: NATS Consumer (depends on p9a-04, p9a-05)

**Wave 6 (Testing - depends on implementation):**
- p9a-08: E2E Tests (depends on p9a-06, p9a-02, p9a-03)
- p9a-09: DLQ Tests (depends on p9a-06)

## Detailed Bead Specifications

### Bead p9a-01 — Scaffold GL Module Skeleton

**Priority:** P0
**Dependencies:** none

**Description:**
Create the `modules/gl` Rust service skeleton (`gl-rs`) with conventional structure and minimal boot that starts HTTP + initializes config/logging/tracing.

**Acceptance Criteria:**
- `gl-rs` compiles in Docker build context (even before wiring DB/NATS)
- Exposes `GET /health` returning 200
- No cross-module imports

**Files to Create/Modify:**
- `modules/gl/Cargo.toml`
- `modules/gl/src/main.rs`
- `modules/gl/src/health.rs`
- `modules/gl/src/config.rs` (env parsing)
- `modules/gl/src/lib.rs` (optional but preferred)
- `modules/gl/README.md`

**Implementation Approach:**
- Follow existing Rust module patterns in repo (Axum if that's the platform standard)
- Config via env: `BUS_TYPE`, `NATS_URL`, `DATABASE_URL`, `PORT=8090`

**How the Agent Should Think:**
"Make the smallest runnable service that future beads can plug into."

**Pitfalls:**
- Pulling shared code from other modules
- Hardcoding localhost ports (must work in Docker network)

---

### Bead p9a-02 — Add GL Postgres + Compose Wiring

**Priority:** P0
**Dependencies:** p9a-01

**Description:**
Add a dedicated Postgres container and ensure network connectivity from `gl-rs`.

**Acceptance Criteria:**
- `7d-gl-postgres` starts with correct DB/user/pass
- `gl-rs` can connect using `DATABASE_URL=postgresql://gl_user:gl_pass@7d-gl-postgres:5432/gl_db`
- Host port mapping `5438:5432` present

**Files to Create/Modify:**
- `infra/docker/docker-compose.*.yml` (where other modules are wired)
- Possibly `.env.docker` or compose env block (repo-dependent)

**Implementation Approach:**
- Mirror existing per-module postgres patterns (container naming, volumes, healthchecks)
- Ensure gl service depends_on gl-postgres health

**How the Agent Should Think:**
"Wire infra first so later beads can run migrations + E2E."

**Pitfalls:**
- Using wrong network / container DNS name
- Forgetting healthcheck → flakey startup order

---

### Bead p9a-03 — Migrations: Journal + Standard Event Tables

**Priority:** P0
**Dependencies:** p9a-02

**Description:**
Implement SQL migrations for:
- `journal_entries`
- `journal_lines`
- `events_outbox`
- `processed_events`
- `failed_events`

**Acceptance Criteria:**
- Migrations apply cleanly on fresh DB
- Constraints exist (PK/FK, non-negative checks, source_event_id unique)
- Journal FK enforces `journal_lines.journal_entry_id → journal_entries.id`

**Files to Create/Modify:**
- `modules/gl/migrations/0001_init.sql` (or sqlx-style timestamped migration dir)
- `modules/gl/migrations/README.md` (optional)

**Implementation Approach:**
- Use same migration tooling as other Rust services (likely `sqlx migrate`)
- Keep "balanced debits == credits" in app layer only

**How the Agent Should Think:**
"Schema must be boring, strict, and match handoff exactly."

**Pitfalls:**
- Adding extra fields not specified
- Forgetting unique constraint on `source_event_id`

---

### Bead p9a-04 — DB Access Layer (sqlx) + Transaction Helpers

**Priority:** P0
**Dependencies:** p9a-03

**Description:**
Add repositories for journal + processed/dlq tables using sqlx and explicit transactions.

**Acceptance Criteria:**
Functions exist for:
- `processed_events.exists(event_id)`
- `processed_events.insert(event_id, subject, tenant_id, processed_at, correlation_id)`
- `journal_entries.insert(...)` returning entry_id
- `journal_lines.bulk_insert(entry_id, lines...)`
- `failed_events.insert(envelope, error, attempt_count, last_attempt_at, ...)`
- All DB writes are transaction-safe for the consumer bead

**Files to Create/Modify:**
- `modules/gl/src/repos/mod.rs`
- `modules/gl/src/repos/journal_repo.rs`
- `modules/gl/src/repos/processed_repo.rs`
- `modules/gl/src/repos/failed_repo.rs`
- `modules/gl/src/db.rs`

**Implementation Approach:**
- Use `sqlx::PgPool`, `sqlx::Transaction<'_, Postgres>`
- Prefer prepared statements via `query!`/`query_as!` if macros are already enabled

**How the Agent Should Think:**
"DB layer is a stable contract for consumer/service logic."

**Pitfalls:**
- Implicit autocommit writes
- Logging full payloads if sensitive (log ids/correlation only)

---

### Bead p9a-05 — Contract Types + Envelope Validation

**Priority:** P0
**Dependencies:** p9a-01 (optionally p9a-04 for db-related validations later)

**Description:**
Implement payload structs for `gl-posting-request.v1.json` and validation that matches schema expectations (without changing schema).

**Acceptance Criteria:**
- Able to deserialize `EventEnvelope<GlPostingRequestV1>`
- Validations:
  - Required fields present
  - At least 2 lines (or allow 1 if schema allows; validate per schema intent)
  - Non-negative amounts
  - Account refs non-empty
  - Currency present

**Files to Create/Modify:**
- `modules/gl/src/contracts/mod.rs`
- `modules/gl/src/contracts/gl_posting_request_v1.rs`
- `modules/gl/src/validation.rs`

**Implementation Approach:**
- If repo already generates types from schemas, follow that mechanism
- Otherwise hand-write serde structs aligned to schema

**How the Agent Should Think:**
"Be strict enough to protect DB integrity; don't invent new contract fields."

**Pitfalls:**
- Drifting from JSON schema shape (field names/casing)
- Over-validating and rejecting valid events

---

### Bead p9a-06 — NATS Consumer: posting.requested → Persist Journal

**Priority:** P0
**Dependencies:** p9a-04, p9a-05

**Description:**
Implement the JetStream/NATS consumer for `gl.events.posting.requested` with idempotency, retries, and DLQ.

**Acceptance Criteria:**
- Consumes subject and processes envelope
- Idempotency via `processed_events`
- Transaction wraps: insert journal_entries, journal_lines, mark processed
- Validates debits == credits
- On validation failure → DLQ after configured retries
- On DB failure → retry with backoff, eventually DLQ
- Tracing span includes correlation_id, tenant_id
- No `.unwrap()` in consumer path

**Files to Create/Modify:**
- `modules/gl/src/consumer/mod.rs`
- `modules/gl/src/consumer/gl_posting_consumer.rs`
- `modules/gl/src/services/journal_service.rs`
- Wire into `main.rs`

**Implementation Approach:**
- Use platform's existing NATS consumer patterns (JetStream durable consumer)
- Use `retry_with_backoff()` from platform utils if available
- Structured error types (Validation vs Retriable vs Fatal)

**How the Agent Should Think:**
"Every event must land in exactly one place: journal or DLQ. Never lost, never duplicated."

**Pitfalls:**
- Acking message before DB commit (message loss)
- Not handling duplicate delivery (double posting)
- Retrying non-retryable validation errors

---

### Bead p9a-07 — Dockerfile + Module Build Integration

**Priority:** P0
**Dependencies:** p9a-01, p9a-02

**Description:**
Ensure `gl-rs` builds/runs Docker-first and integrates with existing build pipeline.

**Acceptance Criteria:**
- `docker compose up` brings up gl service cleanly
- Healthcheck passes
- CI build includes gl-rs (if there's a workspace build step)

**Files to Create/Modify:**
- `modules/gl/Dockerfile`
- Root `Cargo.toml` workspace (if needed)
- CI config if explicit module list exists

**Implementation Approach:**
- Mirror other Rust service Dockerfiles (multi-stage build, minimal runtime image)
- Ensure exposes port 8090 and uses env

**How the Agent Should Think:**
"If it doesn't run in Compose, it doesn't exist."

**Pitfalls:**
- Missing workspace membership
- Copying entire repo into image unnecessarily (slow builds)

---

### Bead p9a-08 — Extend Real Docker E2E: GL Assertions + Idempotency

**Priority:** P0
**Dependencies:** p9a-06, p9a-02, p9a-03

**Description:**
Extend existing E2E flow so after bill-run, we assert GL persistence and duplicate delivery behavior.

**Acceptance Criteria:**
E2E test:
- queries `gl_db.journal_entries` for the invoice source_event_id
- asserts journal_lines exist
- asserts debits == credits
- republishes same envelope/event_id and confirms no second entry
- Test passes in CI with Docker compose

**Files to Create/Modify:**
- Existing E2E test harness (likely under `tools/`, `tests/`, or `infra/docker/e2e/`)
- Add DB query helper for gl postgres (psql/sqlx/pg client)

**Implementation Approach:**
- Prefer black-box: publish event → wait → query DB
- Use deterministic polling with timeout

**How the Agent Should Think:**
"Prove the financial loop closes with real infra."

**Pitfalls:**
- Flaky timing (use polling/backoff)
- Querying wrong DB/port (container vs host confusion)

---

### Bead p9a-09 — DLQ Behavior Test + Observability Touches

**Priority:** P1
**Dependencies:** p9a-06

**Description:**
Validate DLQ write path and ensure tracing/logging is consistent with platform conventions.

**Acceptance Criteria:**
- Test publishes malformed/invalid event → lands in `failed_events`
- Logs include correlation_id, tenant_id, error reason
- Tracing spans propagate correctly
- No panics on bad input

**Files to Create/Modify:**
- Test harness (same as p9a-08 or separate sad-path suite)
- Ensure observability fields set in consumer

**Implementation Approach:**
- Inject bad payload (schema violation, unbalanced entry, etc.)
- Query `failed_events` table for entry

**How the Agent Should Think:**
"DLQ is not failure — it's controlled rejection."

**Pitfalls:**
- DLQ not capturing enough context for debugging
- Logging PII/sensitive data

---

## Implementation Notes

**Critical Path:** p9a-01 → p9a-02 → p9a-03 → p9a-04 → p9a-06 → p9a-08

**Parallelizable:**
- p9a-05 can run parallel to p9a-02/p9a-03
- p9a-07 can run parallel to p9a-03/p9a-04
- p9a-09 can run parallel to p9a-08 (both are testing beads)

**Agent Assignment Strategy:**
- Assign 3 agents to parallel tracks
- Track A: p9a-01 → p9a-02 → p9a-03 → p9a-04
- Track B: p9a-05 (starts after p9a-01 completes)
- Track C: p9a-07 (starts after p9a-02 completes)
- Then converge on p9a-06
- Then split testing: p9a-08 and p9a-09

**Coordination Points:**
- After p9a-01: Kick off p9a-02 and p9a-05
- After p9a-02: Kick off p9a-07
- After p9a-04 and p9a-05 both complete: Kick off p9a-06
- After p9a-06: Kick off p9a-08 and p9a-09

## Screenshot Reference

Full ChatGPT breakdown screenshot saved at: `.phase9-beads-breakdown.png`
