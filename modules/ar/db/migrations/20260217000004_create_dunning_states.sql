-- AR Dunning State Table (bd-1rr)
--
-- Dunning is a deterministic state machine keyed by (app_id, invoice_id).
-- All transitions are atomic with outbox emission (Guard → Mutate → Emit).
--
-- State machine:
--
--   [Pending] ──attempt──> [Warned] ──attempt──> [Escalated]
--       |                     |                       |
--       └──paid──> [Resolved] ◄──────────────────────┘
--       |                                             |
--       └──writeoff──> [WrittenOff]  <────────────────┘
--                                                     |
--                                               [Suspended]
--
-- Schema invariants:
--   1. dunning_id is stable business key (UUID, idempotency anchor)
--   2. One active dunning record per (app_id, invoice_id) — UNIQUE constraint
--   3. Transitions are append-only via `state` + `version` (optimistic locking)
--   4. `version` increments on every transition (compare-and-swap safety)
--   5. Terminal states: Resolved, WrittenOff — no further transitions allowed
--   6. next_attempt_at is NULL for terminal states

CREATE TABLE ar_dunning_states (
    id                  SERIAL          PRIMARY KEY,
    -- Stable business key (idempotency anchor per dunning sequence for an invoice)
    dunning_id          UUID            NOT NULL UNIQUE,
    -- Tenant scoping
    app_id              VARCHAR(50)     NOT NULL,
    -- Invoice this dunning applies to
    invoice_id          INTEGER         NOT NULL REFERENCES ar_invoices(id) ON DELETE RESTRICT,
    -- Customer for denormalized lookups
    customer_id         VARCHAR(255)    NOT NULL,
    -- Current dunning state
    -- pending | warned | escalated | suspended | resolved | written_off
    state               VARCHAR(20)     NOT NULL DEFAULT 'pending',
    -- Monotonic version for optimistic locking (increments on each transition)
    version             INTEGER         NOT NULL DEFAULT 1,
    -- Dunning attempt counter (increments on each attempted collection)
    attempt_count       INTEGER         NOT NULL DEFAULT 0,
    -- When to attempt the next collection (NULL = terminal state)
    next_attempt_at     TIMESTAMPTZ,
    -- Error from the last failed attempt (NULL = no error yet)
    last_error          TEXT,
    -- Outbox event ID of the most recent dunning_state_changed event
    outbox_event_id     UUID,
    -- Timestamps
    created_at          TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ     NOT NULL DEFAULT NOW(),

    -- One active dunning record per invoice per tenant
    CONSTRAINT ar_dunning_states_unique_invoice UNIQUE (app_id, invoice_id)
);

-- Lookup by tenant + state for the scheduler (find invoices due for retry)
CREATE INDEX ar_dunning_states_app_state ON ar_dunning_states(app_id, state);
-- Lookup by next_attempt_at for the dunning scheduler worker
CREATE INDEX ar_dunning_states_next_attempt ON ar_dunning_states(next_attempt_at)
    WHERE next_attempt_at IS NOT NULL;
-- Temporal ordering
CREATE INDEX ar_dunning_states_created_at ON ar_dunning_states(created_at);
