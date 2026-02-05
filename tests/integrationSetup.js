/**
 * Integration Test Setup
 *
 * This file is re-evaluated for each test file (Jest always re-evaluates
 * setupFilesAfterEnv per suite). It provides global lifecycle hooks that
 * ensure each test file starts with a clean database.
 *
 * IMPORTANT: We do NOT call $disconnect between files. Disconnecting and
 * reconnecting Prisma's native query engine between files corrupts internal
 * connection state, causing FK constraint violations even for sequential
 * operations within the same function. The Prisma client persists for the
 * entire test run and disconnects when the process exits.
 *
 * Hook execution order per test file:
 *   1. This beforeAll (cleanDatabase) — wipes leftover data from previous file
 *   2. Test file's beforeAll — creates file-specific fixtures
 *   3. Test file's beforeEach — per-test setup (some files call cleanDatabase here too)
 */

const { cleanDatabase } = require('./integration/database-cleanup');

// Clean database before each test file to prevent cross-file contamination.
// This runs BEFORE the test file's own beforeAll hooks.
beforeAll(async () => {
  await cleanDatabase();
});
