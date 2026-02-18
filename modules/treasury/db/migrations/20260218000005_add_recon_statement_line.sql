-- Add statement_line_id and superseded_by to treasury_recon_matches
-- for statement-line <-> bank-transaction reconciliation with append-only rematch.

ALTER TABLE treasury_recon_matches
    ADD COLUMN statement_line_id UUID REFERENCES treasury_bank_transactions(id) ON DELETE CASCADE,
    ADD COLUMN superseded_by     UUID REFERENCES treasury_recon_matches(id) ON DELETE SET NULL;

-- A statement line may only have ONE active (non-superseded) match
CREATE UNIQUE INDEX treasury_recon_active_match
    ON treasury_recon_matches (statement_line_id)
    WHERE superseded_by IS NULL;

CREATE INDEX treasury_recon_superseded
    ON treasury_recon_matches (superseded_by)
    WHERE superseded_by IS NOT NULL;
