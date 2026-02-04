/**
 * @fireproof/ar - Accounts Receivable module with separate database
 * Customer billing, subscriptions, payments with Tilled integration.
 *
 * Configure with DATABASE_URL_BILLING environment variable
 */

const BillingService = require('./billingService');
const TilledClient = require('./tilledClient');
const billingRoutes = require('./routes/index');
const { captureRawBody, requireAppId, rejectSensitiveData } = require('./middleware');
const handleBillingError = require('./middleware/errorHandler');
const { billingPrisma } = require('./prisma');

module.exports = {
  BillingService,
  TilledClient,
  billingRoutes,
  billingPrisma,
  middleware: {
    captureRawBody,
    requireAppId,
    rejectSensitiveData,
    handleBillingError
  }
};
