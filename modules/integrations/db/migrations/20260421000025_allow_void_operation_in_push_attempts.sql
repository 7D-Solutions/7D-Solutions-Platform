-- Allow 'void' as a valid operation in the push-attempt ledger.
-- Invoices support void via QBO's void API; the HTTP validator now accepts it
-- for entity_type='invoice' only, but the DB constraint is entity-agnostic.
ALTER TABLE integrations_sync_push_attempts
    DROP CONSTRAINT IF EXISTS integrations_sync_push_attempts_operation_check;

ALTER TABLE integrations_sync_push_attempts
    ADD CONSTRAINT integrations_sync_push_attempts_operation_check
        CHECK (operation IN ('create', 'update', 'delete', 'void'));
