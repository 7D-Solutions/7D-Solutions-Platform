const TilledClient = require('./tilledClient');

/**
 * Singleton factory for TilledClient instances.
 * Maintains a per-appId cache so each app gets exactly one client.
 */
const tilledClients = new Map();

function getTilledClient(appId) {
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
