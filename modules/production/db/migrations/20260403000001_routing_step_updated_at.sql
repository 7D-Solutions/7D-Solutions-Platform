-- Add updated_at timestamp to routing_steps for change tracking.
ALTER TABLE routing_steps
    ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
