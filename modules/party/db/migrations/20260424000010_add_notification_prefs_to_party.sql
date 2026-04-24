-- Notification preferences on party_parties (bd-kv15d)
--
-- notification_events: JSON array of event names (e.g. ["shipped","delivered"])
-- notification_channels: JSON array of channel names (e.g. ["email","sms"])
-- GIN indexes enable efficient containment queries over large party sets.

ALTER TABLE party_parties
    ADD COLUMN IF NOT EXISTS notification_events
        JSONB NOT NULL DEFAULT '[]'::jsonb
        CHECK (jsonb_typeof(notification_events) = 'array'),
    ADD COLUMN IF NOT EXISTS notification_channels
        JSONB NOT NULL DEFAULT '[]'::jsonb
        CHECK (jsonb_typeof(notification_channels) = 'array');

CREATE INDEX IF NOT EXISTS party_parties_notification_events_gin
    ON party_parties USING GIN (notification_events);

CREATE INDEX IF NOT EXISTS party_parties_notification_channels_gin
    ON party_parties USING GIN (notification_channels);

COMMENT ON COLUMN party_parties.notification_events IS
    'JSON string array of subscribed shipment events (shipped, out_for_delivery, delivered, exception)';
COMMENT ON COLUMN party_parties.notification_channels IS
    'JSON string array of preferred notification channels (email, sms)';
