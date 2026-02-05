/**
 * Database cleanup strategy for integration tests
 *
 * Uses a SEPARATE raw mysql2 connection for DDL operations (TRUNCATE, SET
 * FOREIGN_KEY_CHECKS). This is intentional — TRUNCATE is DDL that causes
 * implicit commits and can corrupt Prisma's internal connection/transaction
 * state when run through $executeRawUnsafe. A dedicated mysql2 connection
 * avoids polluting the Prisma connection pool.
 *
 * The mysql2 connection is created once and reused across test files.
 * A ping-based health check ensures the connection is alive before use.
 */

const mysql = require('mysql2/promise');

let cleanupConnection = null;

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
 * Parse DATABASE_URL_BILLING into mysql2 connection options.
 */
function parseDbUrl(url) {
  const parsed = new URL(url);
  return {
    host: parsed.hostname,
    port: parseInt(parsed.port, 10) || 3306,
    user: decodeURIComponent(parsed.username),
    password: decodeURIComponent(parsed.password),
    database: parsed.pathname.slice(1).split('?')[0],
  };
}

/**
 * Get or create the mysql2 cleanup connection.
 * Uses ping to verify liveness; reconnects if stale.
 */
async function getCleanupConnection() {
  if (cleanupConnection) {
    try {
      await cleanupConnection.ping();
      return cleanupConnection;
    } catch {
      // Connection went stale — recreate
      cleanupConnection = null;
    }
  }

  const dbUrl = process.env.DATABASE_URL_BILLING;
  if (!dbUrl) {
    throw new Error('DATABASE_URL_BILLING is not set');
  }

  cleanupConnection = await mysql.createConnection(parseDbUrl(dbUrl));
  return cleanupConnection;
}

/**
 * Clean all billing tables using TRUNCATE with FK checks disabled.
 *
 * TRUNCATE resets auto-increment counters for deterministic IDs.
 * FK checks are disabled to avoid constraint errors during cleanup.
 * Uses a raw mysql2 connection to avoid corrupting Prisma's connection state.
 */
async function cleanDatabase() {
  const conn = await getCleanupConnection();

  await conn.execute('SET FOREIGN_KEY_CHECKS = 0');

  for (const table of TABLES) {
    await conn.execute(`TRUNCATE TABLE ${table}`);
  }

  await conn.execute('SET FOREIGN_KEY_CHECKS = 1');
}

/**
 * Setup hook for integration tests.
 * Verifies the test database and cleans all tables.
 */
async function setupIntegrationTests() {
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
 * Teardown hook for integration tests.
 * Closes the cleanup connection if open.
 */
async function teardownIntegrationTests() {
  if (cleanupConnection) {
    await cleanupConnection.end();
    cleanupConnection = null;
  }
}

module.exports = {
  cleanDatabase,
  setupIntegrationTests,
  teardownIntegrationTests
};
