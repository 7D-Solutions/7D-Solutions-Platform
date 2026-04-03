-- Allow notification sends without a template (pre-rendered content).
-- Verticals that render their own HTML can send through the platform
-- without requiring a template_key.

ALTER TABLE notification_sends ALTER COLUMN template_key DROP NOT NULL;
ALTER TABLE notification_sends ALTER COLUMN template_version DROP NOT NULL;
