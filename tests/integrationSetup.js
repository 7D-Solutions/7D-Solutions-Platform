/**
 * Integration Test Setup
 *
 * This file runs AFTER the environment is set up (setupFiles)
 * and ensures Prisma client is fresh for each test suite.
 */

const { resetPrismaClient } = require('../backend/src/prisma.factory');

// Reset Prisma client before each test suite to ensure fresh schema
beforeAll(() => {
  // Reset the cached Prisma client to force recreation with fresh schema
  resetPrismaClient();
});

// NOTE: Global afterEach cleanup removed to avoid race conditions with per-test cleanup.
// Each test file should use database-cleanup.js in its beforeEach hook for consistent cleanup.
