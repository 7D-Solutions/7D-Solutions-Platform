const express = require('express');
const { requireAppId } = require('../middleware');
const RenewalProcessingJob = require('../jobs/RenewalProcessingJob');
const DunningAdvancementJob = require('../jobs/DunningAdvancementJob');
const DataRetentionJob = require('../jobs/DataRetentionJob');

const router = express.Router();
const renewalJob = new RenewalProcessingJob();
const dunningJob = new DunningAdvancementJob();
const dataRetentionJob = new DataRetentionJob();

// POST /api/billing/job-admin/renewal — Trigger renewal processing
router.post('/renewal', requireAppId(), async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const { renewalWindowHours } = req.body || {};

    if (renewalWindowHours) {
      renewalJob.renewalWindowHours = renewalWindowHours;
    }

    const results = await renewalJob.runRenewalJob({ appId });
    res.json(results);
  } catch (error) {
    next(error);
  }
});

// POST /api/billing/job-admin/dunning — Trigger dunning advancement
router.post('/dunning', requireAppId(), async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const results = await dunningJob.runDunningJob({ appId });
    res.json(results);
  } catch (error) {
    next(error);
  }
});

// POST /api/billing/job-admin/data-retention — Trigger data retention
router.post('/data-retention', requireAppId(), async (req, res, next) => {
  try {
    const appId = req.verifiedAppId;
    const { retentionConfig } = req.body || {};

    if (retentionConfig && typeof retentionConfig === 'object') {
      Object.assign(dataRetentionJob.retentionConfig, retentionConfig);
    }

    const results = await dataRetentionJob.runDataRetentionJob({ appId });
    res.json(results);
  } catch (error) {
    next(error);
  }
});

// GET /api/billing/job-admin/status — Get job status (placeholder)
router.get('/status', requireAppId(), async (req, res, next) => {
  try {
    // Could be extended to report last run times, metrics, etc.
    res.json({
      jobs: {
        renewal: { enabled: true, description: 'Renewal invoice generation' },
        dunning: { enabled: true, description: 'Delinquent customer progression' },
        dataRetention: { enabled: true, description: 'Purge/archive old operational data' }
      }
    });
  } catch (error) {
    next(error);
  }
});

module.exports = router;