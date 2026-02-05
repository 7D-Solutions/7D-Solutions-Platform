/**
 * @fireproof/ar - Accounts Receivable module with separate database
 * Customer billing, subscriptions, payments with Tilled integration.
 *
 * Configure with DATABASE_URL_BILLING environment variable
 */

const TilledClient = require('./tilledClient');
const { getTilledClient } = require('./tilledClientFactory');
const billingRoutes = require('./routes/index');
const { captureRawBody, requireAppId, rejectSensitiveData } = require('./middleware');
const handleBillingError = require('./middleware/errorHandler');
const { billingPrisma } = require('./prisma');
const WebhookRetryService = require('./services/WebhookRetryService');
const { RenewalProcessingJob, DunningAdvancementJob, DataRetentionJob } = require('./jobs');

module.exports = {
  TilledClient,
  getTilledClient,
  billingRoutes,
  billingPrisma,
  WebhookRetryService,
  RenewalProcessingJob,
  DunningAdvancementJob,
  DataRetentionJob,
  middleware: {
    captureRawBody,
    requireAppId,
    rejectSensitiveData,
    handleBillingError
  },
  jobs: {
    RenewalProcessingJob,
    DunningAdvancementJob,
    DataRetentionJob
  }
};
