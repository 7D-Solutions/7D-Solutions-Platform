/**
 * Singleton factory for TilledClient instances.
 * Maintains a per-appId cache so each app gets exactly one client.
 *
 * NOTE: TilledClient is required lazily inside getTilledClient() rather than
 * at module scope. This is intentional — with Jest's resetModules: false,
 * a top-level require captures the mock reference from the FIRST test file.
 * Subsequent test files create new mocks via jest.mock(), but a cached
 * top-level reference would still point to the stale mock. Lazy require
 * ensures each call resolves through Jest's current mock registry.
 */
const tilledClients = new Map();

function getTilledClient(appId) {
  const TilledClient = require('./tilledClient');
  // In test mode, always create fresh instances so mocks stay current
  if (process.env.NODE_ENV === 'test') {
    return new TilledClient(appId);
  }
  if (!tilledClients.has(appId)) {
    tilledClients.set(appId, new TilledClient(appId));
  }
  return tilledClients.get(appId);
}

/**
 * Clear the client cache. Called by BillingService constructor
 * so tests that create new BillingService instances in beforeEach
 * get fresh TilledClient instances with current mock implementations.
 */
function clearCache() {
  tilledClients.clear();
}

module.exports = { getTilledClient, clearCache };
