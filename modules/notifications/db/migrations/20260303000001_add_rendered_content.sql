-- Persist rendered template output alongside each delivery attempt.
-- Enables audit, debugging, and idempotent replay without re-rendering.
ALTER TABLE notification_delivery_attempts
    ADD COLUMN rendered_subject TEXT,
    ADD COLUMN rendered_body_html TEXT,
    ADD COLUMN rendered_body_text TEXT;
