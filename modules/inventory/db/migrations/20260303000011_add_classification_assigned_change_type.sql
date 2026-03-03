-- Extend item_change_history change_type check to include classification_assigned.
-- Required for the item classifications feature (bd-2h1ng).

ALTER TABLE item_change_history
    DROP CONSTRAINT item_change_history_change_type_check;

ALTER TABLE item_change_history
    ADD CONSTRAINT item_change_history_change_type_check
    CHECK (change_type IN (
        'revision_created',
        'revision_activated',
        'policy_updated',
        'classification_assigned'
    ));
