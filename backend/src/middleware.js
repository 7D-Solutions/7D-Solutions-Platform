const logger = require('@fireproof/infrastructure/utils/logger');

// Capture raw body for webhook signature verification
// CRITICAL: Must be used BEFORE express.json() middleware
function captureRawBody(req, res, next) {
  req.rawBody = '';
  req.setEncoding('utf8');
  req.on('data', chunk => req.rawBody += chunk);
  req.on('end', () => next());
}

// Validate app_id from auth context (for non-webhook routes)
// Prevents one app from accessing another app's billing data
function requireAppId(options = {}) {
  return (req, res, next) => {
    const requestedAppId = req.params.app_id || req.body.app_id || req.query.app_id;

    if (!requestedAppId) {
      return res.status(400).json({ error: 'Missing app_id' });
    }

    // Extract authorized app_id from JWT/session (only if auth check is configured)
    if (options.getAppIdFromAuth) {
      const authorizedAppId = options.getAppIdFromAuth(req);

      if (!authorizedAppId) {
        return res.status(401).json({ error: 'Unauthorized: No app_id in token' });
      }

      if (authorizedAppId !== requestedAppId) {
        logger.warn('App ID mismatch', {
          authorized: authorizedAppId,
          requested: requestedAppId,
          ip: req.ip
        });
        return res.status(403).json({ error: 'Forbidden: Cannot access other app data' });
      }
    }

    req.verifiedAppId = requestedAppId;
    next();
  };
}

// Reject requests containing raw card/ACH data (PCI safety)
function rejectSensitiveData(req, res, next) {
  const bodyStr = JSON.stringify(req.body).toLowerCase();
  const sensitiveFields = ['card_number', 'card_cvv', 'cvv', 'cvc', 'account_number', 'routing_number'];

  for (const field of sensitiveFields) {
    if (bodyStr.includes(field)) {
      logger.error('PCI violation attempt', { field, ip: req.ip });
      return res.status(400).json({ error: 'PCI violation: Use Tilled hosted fields' });
    }
  }

  next();
}

module.exports = {
  captureRawBody,
  requireAppId,
  rejectSensitiveData
};
