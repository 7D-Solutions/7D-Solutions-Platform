-- Widen amount_minor column from INTEGER (i32) to BIGINT (i64).
-- INTEGER truncates at 2,147,483,647 minor units (~$21.47M).
-- BIGINT supports up to ~$92 quadrillion — no practical limit.
--
-- bd-a924k: BUG: AR amount_cents is i32 — truncates at 21.47M

ALTER TABLE checkout_sessions ALTER COLUMN amount_minor TYPE BIGINT;
