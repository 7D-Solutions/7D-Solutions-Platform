-- Notification preference overrides on party_contacts (bd-kv15d)
--
-- NULL = inherit from parent party record (per-column independent inheritance).
-- Non-null overrides the party-level value for that column only.

ALTER TABLE party_contacts
    ADD COLUMN IF NOT EXISTS notification_events
        JSONB NULL
        CHECK (notification_events IS NULL OR jsonb_typeof(notification_events) = 'array'),
    ADD COLUMN IF NOT EXISTS notification_channels
        JSONB NULL
        CHECK (notification_channels IS NULL OR jsonb_typeof(notification_channels) = 'array');

CREATE INDEX IF NOT EXISTS party_contacts_notification_events_gin
    ON party_contacts USING GIN (notification_events)
    WHERE notification_events IS NOT NULL;

CREATE INDEX IF NOT EXISTS party_contacts_notification_channels_gin
    ON party_contacts USING GIN (notification_channels)
    WHERE notification_channels IS NOT NULL;

COMMENT ON COLUMN party_contacts.notification_events IS
    'Per-contact event override; NULL = inherit from party_parties.notification_events';
COMMENT ON COLUMN party_contacts.notification_channels IS
    'Per-contact channel override; NULL = inherit from party_parties.notification_channels';
