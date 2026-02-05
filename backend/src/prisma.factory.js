/**
 * Prisma Client Factory - Creates fresh Prisma instances
 *
 * This factory pattern ensures tests get fresh Prisma clients
 * without singleton caching issues.
 */

let cachedPrismaClient = null;

function createPrismaClient() {
  // Force require fresh in test environment
  if (process.env.NODE_ENV === 'test') {
    // Delete ALL cached Prisma modules
    Object.keys(require.cache).forEach((key) => {
      if (key.includes('.prisma/ar')) {
        delete require.cache[key];
      }
    });
  }

  const { PrismaClient } = require('../../node_modules/.prisma/ar');

  const opts = {
    datasources: {
      db: {
        url: process.env.DATABASE_URL_BILLING
      }
    }
  };

  // Enable query logging in test to diagnose cross-suite failures
  if (process.env.PRISMA_LOG_QUERIES === '1') {
    opts.log = [
      { level: 'query', emit: 'event' },
      { level: 'error', emit: 'stdout' }
    ];
  }

  const client = new PrismaClient(opts);

  if (process.env.PRISMA_LOG_QUERIES === '1') {
    client.$on('query', (e) => {
      console.log(`[PRISMA] ${e.query} -- params: ${e.params} (${e.duration}ms)`);
    });
  }

  return client;
}

function getBillingPrisma() {
  // Use singleton pattern in both test and production to prevent connection pool exhaustion
  // In test mode, share the same Prisma client across all test files to avoid creating
  // multiple connection pools that compete for database connections
  if (!cachedPrismaClient) {
    cachedPrismaClient = createPrismaClient();
  }
  return cachedPrismaClient;
}

async function resetPrismaClient() {
  if (cachedPrismaClient) {
    await cachedPrismaClient.$disconnect();
    cachedPrismaClient = null;
  }
}

module.exports = {
  getBillingPrisma,
  createPrismaClient,
  resetPrismaClient
};
