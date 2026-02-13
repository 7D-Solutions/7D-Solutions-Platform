-- Add reverses_entry_id column to journal_entries for reversal tracking
-- This enables linking reversal entries back to their original entries

-- Add nullable reverses_entry_id column with self-referencing FK
ALTER TABLE journal_entries
ADD COLUMN reverses_entry_id UUID NULL
REFERENCES journal_entries(id);

-- Add index for reverse entry lookups
CREATE INDEX idx_journal_entries_reverses_entry_id ON journal_entries(reverses_entry_id);

-- Comments for documentation
COMMENT ON COLUMN journal_entries.reverses_entry_id IS 'References the original journal entry that this entry reverses. NULL for non-reversal entries.';
