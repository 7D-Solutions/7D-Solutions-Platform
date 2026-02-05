/**
 * Database cleanup strategy for integration tests
 *
 * Approach: Dedicated test database + TRUNCATE between tests
 * - Each test worker gets a clean slate
 * - Uses DATABASE_URL_BILLING pointing to test database
 * - TRUNCATE all tables (with FK checks disabled) via direct mysql2 connection
 * - TRUNCATE atomically deletes all rows AND resets auto-increment
 *
 * IMPORTANT: We use a raw mysql2 connection instead of Prisma for TRUNCATE.
 * TRUNCATE is DDL (not DML) — it performs an implicit COMMIT and rebuilds the
 * table internally. Running DDL through Prisma's native query engine corrupts
 * its internal connection/transaction state, causing nondeterministic FK
 * constraint violations in subsequent DML operations.
 */

const mysql = require('mysql2/promise');

const TABLES = [
  'billing_divergences',
  'billing_reconciliation_runs',
  'billing_webhook_attempts',
  'billing_events',
  'billing_disputes',
  'billing_refunds',
  'billing_metered_usage',
  'billing_tax_calculations',
  'billing_discount_applications',
  'billing_invoice_line_items',
  'billing_charges',
  'billing_invoices',
  'billing_subscription_addons',
  'billing_addons',
  'billing_payment_methods',
  'billing_subscriptions',
  'billing_plans',
  'billing_coupons',
  'billing_idempotency_keys',
  'billing_webhooks',
  'billing_tax_rates',
  'billing_customers',
];

/**
 * Parse DATABASE_URL_BILLING into mysql2 connection config.
 * Format: mysql://user:pass@host:port/database?params
 */
function parseDatabaseUrl(url) {
  const parsed = new URL(url);
  return {
    host: parsed.hostname,
    port: parseInt(parsed.port, 10) || 3306,
    user: parsed.username,
    password: parsed.password,
    database: parsed.pathname.slice(1), // remove leading /
  };
}

/** Lazily-created connection, reused for the lifetime of the test process. */
let _conn = null;

async function getConnection() {
  if (_conn) return _conn;
  const config = parseDatabaseUrl(process.env.DATABASE_URL_BILLING);
  _conn = await mysql.createConnection(config);
  return _conn;
}

/**
 * Clean all billing tables using TRUNCATE with FK checks disabled.
 *
 * Uses a direct mysql2 connection to avoid corrupting Prisma's query engine
 * with DDL operations.
 */
async function cleanDatabase() {
  const conn = await getConnection();

  await conn.query('SET FOREIGN_KEY_CHECKS = 0');
  for (const table of TABLES) {
    await conn.query(`TRUNCATE TABLE ${table}`);
  }
  await conn.query('SET FOREIGN_KEY_CHECKS = 1');
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
  // No-op: Prisma client persists for the entire test run.
  // The client disconnects automatically when the process exits.
}

module.exports = {
  cleanDatabase,
  setupIntegrationTests,
  teardownIntegrationTests
};
