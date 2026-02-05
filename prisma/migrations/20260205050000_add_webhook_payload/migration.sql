-- AlterTable
ALTER TABLE `billing_webhooks` ADD COLUMN `payload` JSON NULL AFTER `error`;
