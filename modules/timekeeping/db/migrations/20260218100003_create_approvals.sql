-- Timekeeping: Approval Workflow
--
-- Timesheets are approved per employee per period (e.g. weekly).
-- An approval_request represents the submission of a time period for review.
-- Each status change is tracked as an approval_action for audit trail.
--
-- Flow: draft → submitted → approved / rejected → (recall → draft)

CREATE TYPE tk_approval_status AS ENUM (
    'draft',       -- not yet submitted
    'submitted',   -- awaiting review
    'approved',    -- approved by reviewer
    'rejected'     -- sent back for corrections
);

CREATE TABLE tk_approval_requests (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id         VARCHAR(50) NOT NULL,
    employee_id    UUID NOT NULL REFERENCES tk_employees(id) ON DELETE RESTRICT,
    period_start   DATE NOT NULL,
    period_end     DATE NOT NULL,
    status         tk_approval_status NOT NULL DEFAULT 'draft',
    -- Total minutes in the submitted period (denormalized for display)
    total_minutes  INT NOT NULL DEFAULT 0,
    submitted_at   TIMESTAMPTZ,
    reviewed_at    TIMESTAMPTZ,
    reviewer_id    UUID,                    -- who approved/rejected
    reviewer_notes TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- One approval request per employee per period
    CONSTRAINT tk_approvals_employee_period_unique
        UNIQUE (app_id, employee_id, period_start, period_end)
);

CREATE INDEX tk_approvals_app_id
    ON tk_approval_requests(app_id);
CREATE INDEX tk_approvals_employee
    ON tk_approval_requests(app_id, employee_id);
CREATE INDEX tk_approvals_status
    ON tk_approval_requests(app_id, status);
CREATE INDEX tk_approvals_period
    ON tk_approval_requests(app_id, period_start, period_end);

-- ============================================================
-- APPROVAL ACTIONS (audit trail for status transitions)
-- ============================================================

CREATE TABLE tk_approval_actions (
    id           BIGSERIAL PRIMARY KEY,
    approval_id  UUID NOT NULL REFERENCES tk_approval_requests(id) ON DELETE CASCADE,
    action       VARCHAR(50) NOT NULL,      -- 'submit', 'approve', 'reject', 'recall'
    actor_id     UUID NOT NULL,             -- who performed the action
    notes        TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX tk_approval_actions_approval_id
    ON tk_approval_actions(approval_id);
CREATE INDEX tk_approval_actions_actor
    ON tk_approval_actions(actor_id);
