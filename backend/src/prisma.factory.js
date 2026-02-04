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

  const client = new PrismaClient({
    datasources: {
      db: {
        url: process.env.DATABASE_URL_BILLING
      }
    }
  });

  return client;
}

function getBillingPrisma() {
  // In test mode, always create a fresh client to avoid caching issues
  if (process.env.NODE_ENV === 'test') {
    return createPrismaClient();
  }

  // In production, use cached client
  if (!cachedPrismaClient) {
    cachedPrismaClient = createPrismaClient();
  }
  return cachedPrismaClient;
}

function resetPrismaClient() {
  if (cachedPrismaClient) {
    cachedPrismaClient.$disconnect();
    cachedPrismaClient = null;
  }
}

module.exports = {
  getBillingPrisma,
  createPrismaClient,
  resetPrismaClient
};
