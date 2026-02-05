const express = require('express');
const healthRoutes = require('./healthRoutes');
const customerRoutes = require('./customerRoutes');
const billingStateRoutes = require('./billingStateRoutes');
const paymentMethodRoutes = require('./paymentMethodRoutes');
const subscriptionRoutes = require('./subscriptionRoutes');
const webhookRoutes = require('./webhookRoutes');
const chargeRoutes = require('./chargeRoutes');
const refundRoutes = require('./refundRoutes');
const taxRoutes = require('./taxRoutes');
const prorationRoutes = require('./prorationRoutes');
const usageRoutes = require('./usageRoutes');
const invoiceRoutes = require('./invoiceRoutes');
const webhookAdminRoutes = require('./webhookAdminRoutes');
const jobAdminRoutes = require('./jobAdminRoutes');

const router = express.Router();

// Webhook route must be mounted BEFORE requireAppId middleware
router.use('/webhooks', webhookRoutes);

// All other routes use requireAppId middleware (applied at mount time)
router.use('/health', healthRoutes);
router.use('/customers', customerRoutes);
router.use('/state', billingStateRoutes);
router.use('/payment-methods', paymentMethodRoutes);
router.use('/subscriptions', subscriptionRoutes);
router.use('/charges', chargeRoutes);
router.use('/refunds', refundRoutes);
router.use('/tax', taxRoutes);
router.use('/proration', prorationRoutes);
router.use('/usage', usageRoutes);
router.use('/invoices', invoiceRoutes);
router.use('/webhook-admin', webhookAdminRoutes);
router.use('/job-admin', jobAdminRoutes);

module.exports = router;