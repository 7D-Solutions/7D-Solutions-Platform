/**
 * Integration Test Setup
 *
 * This file runs AFTER the environment is set up (setupFiles)
 * and ensures Prisma client is fresh for each test suite.
 */

// NOTE: resetPrismaClient() removed — each test file gets a fresh module registry
// which creates a fresh Prisma client. Resetting disconnects the client that modules
// already captured, causing race conditions with subsequent queries.

// NOTE: Global afterEach cleanup removed to avoid race conditions with per-test cleanup.
// Each test file should use database-cleanup.js in its beforeEach hook for consistent cleanup.
