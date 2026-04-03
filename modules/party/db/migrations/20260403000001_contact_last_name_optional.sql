-- Make last_name optional on party_contacts.
-- Most systems treat last_name as optional (mononyms, organizations, etc.).
ALTER TABLE party_contacts ALTER COLUMN last_name DROP NOT NULL;
