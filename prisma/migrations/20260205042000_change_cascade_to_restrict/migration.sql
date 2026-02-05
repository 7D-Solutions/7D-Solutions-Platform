-- Change foreign key constraints from CASCADE to RESTRICT for financial record retention
-- Compliance requirement: Financial records must be preserved for 7+ years (tax, audit, PCI DSS)
-- Prevents accidental deletion of customer from destroying financial history

-- billing_subscriptions.billing_customer_id
ALTER TABLE `billing_subscriptions` DROP FOREIGN KEY `billing_subscriptions_billing_customer_id_fkey`;
ALTER TABLE `billing_subscriptions` ADD CONSTRAINT `billing_subscriptions_billing_customer_id_fkey` FOREIGN KEY (`billing_customer_id`) REFERENCES `billing_customers`(`id`) ON DELETE RESTRICT ON UPDATE CASCADE;

-- billing_payment_methods.billing_customer_id
ALTER TABLE `billing_payment_methods` DROP FOREIGN KEY `billing_payment_methods_billing_customer_id_fkey`;
ALTER TABLE `billing_payment_methods` ADD CONSTRAINT `billing_payment_methods_billing_customer_id_fkey` FOREIGN KEY (`billing_customer_id`) REFERENCES `billing_customers`(`id`) ON DELETE RESTRICT ON UPDATE CASCADE;

-- billing_invoices.billing_customer_id
ALTER TABLE `billing_invoices` DROP FOREIGN KEY `billing_invoices_billing_customer_id_fkey`;
ALTER TABLE `billing_invoices` ADD CONSTRAINT `billing_invoices_billing_customer_id_fkey` FOREIGN KEY (`billing_customer_id`) REFERENCES `billing_customers`(`id`) ON DELETE RESTRICT ON UPDATE CASCADE;

-- billing_charges.billing_customer_id
ALTER TABLE `billing_charges` DROP FOREIGN KEY `billing_charges_billing_customer_id_fkey`;
ALTER TABLE `billing_charges` ADD CONSTRAINT `billing_charges_billing_customer_id_fkey` FOREIGN KEY (`billing_customer_id`) REFERENCES `billing_customers`(`id`) ON DELETE RESTRICT ON UPDATE CASCADE;

-- billing_refunds.charge_id (cascade on charge deletion → restrict)
ALTER TABLE `billing_refunds` DROP FOREIGN KEY `billing_refunds_charge_id_fkey`;
ALTER TABLE `billing_refunds` ADD CONSTRAINT `billing_refunds_charge_id_fkey` FOREIGN KEY (`charge_id`) REFERENCES `billing_charges`(`id`) ON DELETE RESTRICT ON UPDATE CASCADE;

-- billing_disputes.charge_id (cascade on charge deletion → restrict)
ALTER TABLE `billing_disputes` DROP FOREIGN KEY `billing_disputes_charge_id_fkey`;
ALTER TABLE `billing_disputes` ADD CONSTRAINT `billing_disputes_charge_id_fkey` FOREIGN KEY (`charge_id`) REFERENCES `billing_charges`(`id`) ON DELETE RESTRICT ON UPDATE CASCADE;

-- billing_subscription_addons.subscription_id
ALTER TABLE `billing_subscription_addons` DROP FOREIGN KEY `billing_subscription_addons_subscription_id_fkey`;
ALTER TABLE `billing_subscription_addons` ADD CONSTRAINT `billing_subscription_addons_subscription_id_fkey` FOREIGN KEY (`subscription_id`) REFERENCES `billing_subscriptions`(`id`) ON DELETE RESTRICT ON UPDATE CASCADE;

-- billing_subscription_addons.addon_id
ALTER TABLE `billing_subscription_addons` DROP FOREIGN KEY `billing_subscription_addons_addon_id_fkey`;
ALTER TABLE `billing_subscription_addons` ADD CONSTRAINT `billing_subscription_addons_addon_id_fkey` FOREIGN KEY (`addon_id`) REFERENCES `billing_addons`(`id`) ON DELETE RESTRICT ON UPDATE CASCADE;

-- billing_tax_calculations.invoice_id (cascade → restrict)
ALTER TABLE `billing_tax_calculations` DROP FOREIGN KEY `billing_tax_calculations_invoice_id_fkey`;
ALTER TABLE `billing_tax_calculations` ADD CONSTRAINT `billing_tax_calculations_invoice_id_fkey` FOREIGN KEY (`invoice_id`) REFERENCES `billing_invoices`(`id`) ON DELETE RESTRICT ON UPDATE CASCADE;

-- billing_tax_calculations.charge_id (cascade → restrict)
ALTER TABLE `billing_tax_calculations` DROP FOREIGN KEY `billing_tax_calculations_charge_id_fkey`;
ALTER TABLE `billing_tax_calculations` ADD CONSTRAINT `billing_tax_calculations_charge_id_fkey` FOREIGN KEY (`charge_id`) REFERENCES `billing_charges`(`id`) ON DELETE RESTRICT ON UPDATE CASCADE;

-- Note: Foreign keys with ON DELETE SET NULL remain unchanged (e.g., invoice.subscription_id, charge.subscription_id, charge.invoice_id, discount_applications.*, metered_usage.subscription_id)
-- These allow parent record deletion while preserving child records with null foreign key.
-- Note: billing_divergences.run_id retains ON DELETE CASCADE (internal audit table).
-- Note: billing_metered_usage.customer_id already RESTRICT, billing_invoice_line_items.invoice_id already RESTRICT, billing_tax_calculations.tax_rate_id already RESTRICT.