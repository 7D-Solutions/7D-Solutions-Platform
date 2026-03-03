-- Timekeeping: Clock In/Out Sessions
--
-- Tracks employee clock-in/clock-out sessions for payroll and compliance.
-- Key invariant: no concurrent open sessions per employee within a tenant.
--
-- Status: 'open' = clocked in, 'closed' = clocked out.
-- Duration stored in whole minutes, computed on clock-out.

-- ============================================================
-- CLOCK SESSIONS
-- ============================================================

CREATE TABLE tk_clock_sessions (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id            VARCHAR(50) NOT NULL,
    employee_id       UUID NOT NULL REFERENCES tk_employees(id) ON DELETE RESTRICT,
    clock_in_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    clock_out_at      TIMESTAMPTZ,
    duration_minutes  INTEGER,
    status            VARCHAR(10) NOT NULL DEFAULT 'open'
                      CHECK (status IN ('open', 'closed')),
    idempotency_key   VARCHAR(255),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Prevent duplicate idempotency keys per tenant
    CONSTRAINT tk_clock_sessions_idempotency_unique
        UNIQUE (app_id, idempotency_key)
);

-- Partial unique index: at most one open session per employee per tenant.
-- This is the DB-level enforcement of the concurrent session guard.
CREATE UNIQUE INDEX tk_clock_sessions_one_open
    ON tk_clock_sessions (app_id, employee_id)
    WHERE status = 'open';

-- Tenant query index
CREATE INDEX tk_clock_sessions_app_id
    ON tk_clock_sessions (app_id);

-- Employee lookup index
CREATE INDEX tk_clock_sessions_employee
    ON tk_clock_sessions (app_id, employee_id, status);
