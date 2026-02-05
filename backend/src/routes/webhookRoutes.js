const express = require('express');
const { getTilledClient } = require('../tilledClientFactory');
const WebhookService = require('../services/WebhookService');
const logger = require('@fireproof/infrastructure/utils/logger');

const router = express.Router();
const webhookService = new WebhookService(getTilledClient);

// POST /api/billing/webhooks/:app_id (NO auth middleware - signature verification only)
router.post('/:app_id', async (req, res, next) => {
  try {
    const { app_id } = req.params;
    const signature = req.headers['payments-signature'];
    const rawBody = req.rawBody;
    const event = req.body;

    if (!signature) {
      return res.status(401).json({ error: 'Missing webhook signature' });
    }

    if (!rawBody) {
      logger.error('Missing rawBody - captureRawBody middleware not configured');
      return res.status(500).json({ error: 'Server configuration error' });
    }

    const result = await webhookService.processWebhook(app_id, event, rawBody, signature);

    if (!result.success) {
      return res.status(401).json({ error: result.error || 'Invalid webhook signature' });
    }

    res.json({ received: true, duplicate: result.duplicate || false });
  } catch (error) {
    next(error);
  }
});

module.exports = router;