const express = require('express');
const { billingPrisma } = require('../prisma');
const { requireAppId } = require('../middleware');

const router = express.Router();

// GET /api/billing/health (admin-only health check)
router.get('/', requireAppId(), async (req, res) => {
  const appId = req.verifiedAppId;

  const checks = {
    timestamp: new Date().toISOString(),
    app_id: appId,
    database: { status: 'unknown', error: null },
    tilled_config: { status: 'unknown', error: null }
  };

  // Check database connectivity
  try {
    await billingPrisma.$queryRaw`SELECT 1`;
    checks.database.status = 'healthy';
  } catch (error) {
    checks.database.status = 'unhealthy';
    checks.database.error = error.message;
  }

  // Check Tilled credentials present
  try {
    const prefix = appId.toUpperCase();
    const secretKey = process.env[`TILLED_SECRET_KEY_${prefix}`];
    const accountId = process.env[`TILLED_ACCOUNT_ID_${prefix}`];
    const webhookSecret = process.env[`TILLED_WEBHOOK_SECRET_${prefix}`];
    const sandbox = process.env.TILLED_SANDBOX;

    const missing = [];
    if (!secretKey) missing.push('TILLED_SECRET_KEY');
    if (!accountId) missing.push('TILLED_ACCOUNT_ID');
    if (!webhookSecret) missing.push('TILLED_WEBHOOK_SECRET');
    if (sandbox === undefined) missing.push('TILLED_SANDBOX');

    if (missing.length > 0) {
      checks.tilled_config.status = 'unhealthy';
      checks.tilled_config.error = `Missing credentials: ${missing.join(', ')}`;
    } else {
      checks.tilled_config.status = 'healthy';
      checks.tilled_config.sandbox_mode = sandbox === 'true';
    }
  } catch (error) {
    checks.tilled_config.status = 'unhealthy';
    checks.tilled_config.error = error.message;
  }

  // Overall health
  const allHealthy = checks.database.status === 'healthy' &&
                     checks.tilled_config.status === 'healthy';

  const statusCode = allHealthy ? 200 : 503;
  checks.overall_status = allHealthy ? 'healthy' : 'degraded';

  res.status(statusCode).json(checks);
});

module.exports = router;