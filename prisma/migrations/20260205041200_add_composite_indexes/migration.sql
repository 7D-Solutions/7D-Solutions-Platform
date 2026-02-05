-- Composite index for subscription renewal queries (WHERE app_id = ? AND status = ? AND current_period_end < ?)
CREATE INDEX `idx_app_status_period_end` ON `billing_subscriptions`(`app_id`, `status`, `current_period_end`);

-- Composite index for aging receivables queries (WHERE app_id = ? AND status = ? ORDER BY due_at)
CREATE INDEX `idx_app_status_due_at` ON `billing_invoices`(`app_id`, `status`, `due_at`);

-- Composite index for revenue reporting queries (WHERE app_id = ? ORDER BY created_at)
CREATE INDEX `idx_app_created_at` ON `billing_charges`(`app_id`, `created_at`);
