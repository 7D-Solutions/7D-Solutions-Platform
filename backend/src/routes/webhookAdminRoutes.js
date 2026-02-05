const express = require('express');
const { getTilledClient } = require('../tilledClientFactory');
const { requireAppId } = require('../middleware');
const WebhookService = require('../services/WebhookService');
const WebhookRetryService = require('../services/WebhookRetryService');

const router = express.Router();
const webhookService = new WebhookService(getTilledClient);
const retryService = new WebhookRetryService(webhookService);

// POST /api/billing/webhook-admin/retry — Trigger retry processing for the app
router.post('/retry', requireAppId(), async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const { batchSize, maxAttempts } = req.body || {};

    const results = await retryService.processRetries({
      appId,
      ...(batchSize && { batchSize }),
      ...(maxAttempts && { maxAttempts })
    });

    res.json({ processed: results.length, results });
  } catch (error) {
    next(error);
  }
});

// GET /api/billing/webhook-admin/stats — Get retry queue stats
router.get('/stats', requireAppId(), async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const stats = await retryService.getRetryStats(appId);
    res.json(stats);
  } catch (error) {
    next(error);
  }
});

// POST /api/billing/webhook-admin/retry/:event_id — Manually retry a dead-lettered webhook
router.post('/retry/:event_id', requireAppId(), async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const { event_id } = req.params;

    const result = await retryService.retryDeadLetter(appId, event_id);
    res.json(result);
  } catch (error) {
    next(error);
  }
});

module.exports = router;
