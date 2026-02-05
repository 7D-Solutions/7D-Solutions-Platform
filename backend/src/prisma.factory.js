/**
 * Prisma Client Factory
 *
 * Module-level singleton caching. With `resetModules: false` in Jest's
 * integration project config, the module registry persists across test files,
 * so all files share the same PrismaClient and connection pool. This prevents
 * connection pool exhaustion and ensures consistent database state visibility.
 *
 * IMPORTANT: Do NOT use globalThis for caching — Jest creates separate
 * vm sandboxes per test file, so globalThis is NOT shared. Module-level
 * variables ARE shared when resetModules is false.
 */

let cachedPrismaClient = null;

function createPrismaClient() {
  const { PrismaClient } = require('../../node_modules/.prisma/ar');

  const opts = {
    datasources: {
      db: {
        url: process.env.DATABASE_URL_BILLING
      }
    }
  };

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
