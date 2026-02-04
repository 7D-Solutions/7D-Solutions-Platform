/**
 * Database cleanup strategy for integration tests
 *
 * Approach: Dedicated test database + truncate between tests
 * - Each test worker gets a clean slate
 * - Uses DATABASE_URL_BILLING pointing to test database
 * - beforeEach: truncate all tables (fast, preserves schema)
 * - afterAll: disconnect Prisma client
 */

const { billingPrisma } = require('../../backend/src/prisma');

/**
 * Clean all billing tables (order matters due to foreign keys)
 * Wrapped in transaction to prevent deadlocks during parallel test execution
 */
async function cleanDatabase() {
  // Use transaction to execute all deletes atomically
  // This prevents write conflicts and deadlocks when tests run in parallel
  await billingPrisma.$transaction(async (tx) => {
    // Delete in reverse dependency order (children first, then parents)

    // Divergences (child of reconciliation_runs)
    await tx.billing_divergences.deleteMany({});

    // Reconciliation runs
    await tx.billing_reconciliation_runs.deleteMany({});

    // Webhook attempts
    await tx.billing_webhook_attempts.deleteMany({});

    // Events log
    await tx.billing_events.deleteMany({});

    // Most dependent children (references charges)
    await tx.billing_disputes.deleteMany({});
    await tx.billing_refunds.deleteMany({});

    // Phase 4: Metered usage (references customers, subscriptions)
    await tx.billing_metered_usage.deleteMany({});

    // Phase 1-2-3: Tax calculations, discount applications, invoice line items
    await tx.billing_tax_calculations.deleteMany({});
    await tx.billing_discount_applications.deleteMany({});
    await tx.billing_invoice_line_items.deleteMany({});

    // Charges (references invoices, customers, subscriptions)
    await tx.billing_charges.deleteMany({});

    // Invoices (references customers, subscriptions)
    await tx.billing_invoices.deleteMany({});

    // Subscription addons (references subscriptions, addons)
    await tx.billing_subscription_addons.deleteMany({});

    // Addons (parent of subscription_addons)
    await tx.billing_addons.deleteMany({});

    // Payment methods (references customers)
    await tx.billing_payment_methods.deleteMany({});

    // Subscriptions (references customers)
    await tx.billing_subscriptions.deleteMany({});

    // Independent tables (no FK constraints)
    await tx.billing_plans.deleteMany({});
    await tx.billing_coupons.deleteMany({});
    await tx.billing_idempotency_keys.deleteMany({});
    await tx.billing_webhooks.deleteMany({});
    await tx.billing_tax_rates.deleteMany({});

    // Parent tables
    await tx.billing_customers.deleteMany({});
  });
}

/**
 * Setup hook for integration tests
 */
async function setupIntegrationTests() {
  // Verify we're using test database
  const dbUrl = process.env.DATABASE_URL_BILLING;
  if (!dbUrl || !dbUrl.includes('_test')) {
    throw new Error(
      'DATABASE_URL_BILLING must point to test database (name must contain "_test")\n' +
      `Current: ${dbUrl}`
    );
  }

  await cleanDatabase();
}

/**
 * Teardown hook for integration tests
 */
async function teardownIntegrationTests() {
  await billingPrisma.$disconnect();
}

module.exports = {
  cleanDatabase,
  setupIntegrationTests,
  teardownIntegrationTests
};
