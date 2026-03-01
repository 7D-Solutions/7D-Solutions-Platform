-- Add PAST_DUE and SUSPENDED states to subscription status
-- Part of Phase 15 bd-138: Subscriptions Transition Guards

-- Drop the old CHECK constraint
ALTER TABLE subscriptions DROP CONSTRAINT IF EXISTS subscriptions_status_check;

-- Add new CHECK constraint with lifecycle states
ALTER TABLE subscriptions ADD CONSTRAINT subscriptions_status_check
    CHECK (status IN ('active', 'past_due', 'suspended', 'paused', 'cancelled'));

-- Note: 'paused' and 'cancelled' are retained for backward compatibility
-- Phase 15 focuses on: ACTIVE, PAST_DUE, SUSPENDED
