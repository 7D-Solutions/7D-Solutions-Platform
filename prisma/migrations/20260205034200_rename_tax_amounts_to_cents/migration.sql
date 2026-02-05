-- Rename tax amount columns from Decimal dollars to Int cents for consistency
-- All other tables in the billing schema use integer cents (e.g., amount_cents, price_cents)

-- Step 1: Add new Int columns
ALTER TABLE `billing_tax_calculations` ADD COLUMN `taxable_amount_cents` INT NOT NULL DEFAULT 0;
ALTER TABLE `billing_tax_calculations` ADD COLUMN `tax_amount_cents` INT NOT NULL DEFAULT 0;

-- Step 2: Migrate existing data (convert dollars to cents)
UPDATE `billing_tax_calculations`
SET `taxable_amount_cents` = ROUND(`taxable_amount` * 100),
    `tax_amount_cents` = ROUND(`tax_amount` * 100);

-- Step 3: Drop old Decimal columns
ALTER TABLE `billing_tax_calculations` DROP COLUMN `taxable_amount`;
ALTER TABLE `billing_tax_calculations` DROP COLUMN `tax_amount`;
