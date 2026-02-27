-- Add low-code lifecycle statuses for webhook-driven reconciliation.
-- Routes now record local intent only; provider state arrives via webhooks.

-- Add pending_sync and canceling to subscription status enum
ALTER TYPE ar_subscriptions_status ADD VALUE IF NOT EXISTS 'pending_sync';
ALTER TYPE ar_subscriptions_status ADD VALUE IF NOT EXISTS 'canceling';

-- Allow NULL tilled_subscription_id (set by webhook, not route)
ALTER TABLE ar_subscriptions ALTER COLUMN tilled_subscription_id DROP NOT NULL;
