/**
 * AR Prisma Client - Separate database from main application
 *
 * IMPORTANT: This is a completely separate Prisma client from the main app.
 * Generated from packages/ar/prisma/schema.prisma
 * Output: node_modules/.prisma/ar
 *
 * Configure with DATABASE_URL_BILLING environment variable
 * This client ONLY accesses billing tables (billing_customers, billing_subscriptions, billing_webhooks)
 * It NEVER touches main app tables (customers, gauges, quotes, etc.)
 */

// Use factory pattern to ensure fresh Prisma client in tests
const { getBillingPrisma } = require('./prisma.factory');

const billingPrisma = getBillingPrisma();

module.exports = { billingPrisma };
