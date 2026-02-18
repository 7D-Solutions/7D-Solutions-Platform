-- Performance: partial composite index for dunning scheduler claim query (bd-1was)
--
-- The scheduler's claim query filters by (next_attempt_at <= now, state NOT IN terminal),
-- orders by next_attempt_at ASC, and uses SKIP LOCKED. A partial index excluding
-- terminal states reduces the index size and makes the scan selective.
--
-- Existing indexes: ar_dunning_states_next_attempt (next_attempt_at partial WHERE NOT NULL)
-- This index adds state filtering to avoid re-evaluating the state predicate at runtime.

CREATE INDEX IF NOT EXISTS ar_dunning_states_active_attempt
    ON ar_dunning_states(next_attempt_at ASC)
    WHERE state NOT IN ('resolved', 'written_off') AND next_attempt_at IS NOT NULL;
