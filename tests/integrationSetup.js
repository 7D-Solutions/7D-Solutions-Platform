/**
 * Integration Test Setup
 *
 * This file is re-evaluated for each test file (Jest always re-evaluates
 * setupFilesAfterEnv per suite).
 *
 * IMPORTANT: We do NOT call $disconnect between files. Disconnecting and
 * reconnecting Prisma's native query engine between files corrupts internal
 * connection state, causing FK constraint violations even for sequential
 * operations within the same function. The Prisma client persists for the
 * entire test run and disconnects when the process exits.
 */

const { cleanDatabase } = require('./integration/database-cleanup');

beforeAll(async () => {
  await cleanDatabase();
});
