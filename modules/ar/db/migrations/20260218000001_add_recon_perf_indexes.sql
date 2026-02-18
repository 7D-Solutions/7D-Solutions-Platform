-- Performance: composite index for AR recon charge lookup (bd-h9et)
--
-- The recon engine queries ar_charges with (app_id, status) together.
-- A composite index outperforms the existing separate (app_id) and (status)
-- indexes for this query shape, enabling a more selective index scan.
--
-- ar_invoices already has ar_invoices_app_status_due_at covering (app_id, status).

CREATE INDEX IF NOT EXISTS ar_charges_app_status ON ar_charges(app_id, status);
