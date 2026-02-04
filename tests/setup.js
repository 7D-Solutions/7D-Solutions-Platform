/**
 * Test setup for @fireproof/ar
 *
 * CRITICAL: Environment variables must be set BEFORE any imports happen
 * This file runs via setupFilesAfterEnv, but env vars are needed earlier
 */

// Load environment file FIRST
require('dotenv').config({ path: require('path').resolve(__dirname, '../../../.env') });

// FORCE test environment variables (override .env values)
// Use port 3309 for trashtech-mysql container (maps to internal 3306)
process.env.DATABASE_URL_BILLING = 'mysql://billing_test:testpass@localhost:3309/billing_test';

// Always set mock Tilled credentials for tests
process.env.TILLED_SECRET_KEY_TRASHTECH = 'sk_test_mock';
process.env.TILLED_ACCOUNT_ID_TRASHTECH = 'acct_mock';
process.env.TILLED_WEBHOOK_SECRET_TRASHTECH = 'whsec_mock';
process.env.TILLED_SANDBOX = 'true';

// Mock logger to prevent console spam during tests
jest.mock('@fireproof/infrastructure/utils/logger', () => ({
  info: jest.fn(),
  warn: jest.fn(),
  error: jest.fn(),
  debug: jest.fn()
}));

// Increase timeout for database operations
jest.setTimeout(10000);
