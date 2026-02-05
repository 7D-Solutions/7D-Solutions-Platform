const crypto = require('crypto');

class TilledClient {
  constructor(appId) {
    this.appId = appId;
    this.config = this.loadConfig(appId);
    this.initialized = false;
  }

  loadConfig(appId) {
    const prefix = appId.toUpperCase();
    const secretKey = process.env[`TILLED_SECRET_KEY_${prefix}`];
    const accountId = process.env[`TILLED_ACCOUNT_ID_${prefix}`];
    const webhookSecret = process.env[`TILLED_WEBHOOK_SECRET_${prefix}`];
    const sandbox = process.env.TILLED_SANDBOX === 'true';

    if (!secretKey || !accountId || !webhookSecret) {
      throw new Error(`Missing Tilled config for app: ${appId}`);
    }

    return {
      secretKey,
      accountId,
      webhookSecret,
      sandbox,
      basePath: sandbox ? 'https://sandbox-api.tilled.com' : 'https://api.tilled.com'
    };
  }

  initializeSDK() {
    if (this.initialized) return;

    const tilled = require('tilled-node');
    const sdkConfig = new tilled.Configuration({
      apiKey: this.config.secretKey,
      basePath: this.config.basePath
    });

    this.customersApi = new tilled.CustomersApi(sdkConfig);
    this.subscriptionsApi = new tilled.SubscriptionsApi(sdkConfig);
    this.paymentMethodsApi = new tilled.PaymentMethodsApi(sdkConfig);
    // Initialize RefundsApi and DisputesApi if available (may not exist in test mocks)
    if (tilled.RefundsApi) {
      this.refundsApi = new tilled.RefundsApi(sdkConfig);
    }
    if (tilled.DisputesApi) {
      this.disputesApi = new tilled.DisputesApi(sdkConfig);
    }
    this.initialized = true;
  }

  async createCustomer(email, name, metadata = {}) {
    this.initializeSDK();
    const response = await this.customersApi.createCustomer(this.config.accountId, {
      email,
      first_name: name,
      metadata
    });
    return response.data;
  }

  async attachPaymentMethod(paymentMethodId, customerId) {
    this.initializeSDK();
    const response = await this.paymentMethodsApi.attachPaymentMethodToCustomer(
      this.config.accountId,
      paymentMethodId,
      { customer_id: customerId }
    );
    return response.data;
  }

  async detachPaymentMethod(paymentMethodId) {
    this.initializeSDK();
    const response = await this.paymentMethodsApi.detachPaymentMethodFromCustomer(
      this.config.accountId,
      paymentMethodId
    );
    return response.data;
  }

  async getPaymentMethod(paymentMethodId) {
    this.initializeSDK();
    const response = await this.paymentMethodsApi.getPaymentMethod(
      this.config.accountId,
      paymentMethodId
    );
    return response.data;
  }

  async listPaymentMethods(customerId) {
    this.initializeSDK();
    const response = await this.paymentMethodsApi.listPaymentMethods(
      this.config.accountId,
      { customer_id: customerId }
    );
    return response.data;
  }

  async createSubscription(customerId, paymentMethodId, priceCents, options = {}) {
    this.initializeSDK();
    const response = await this.subscriptionsApi.createSubscription(this.config.accountId, {
      customer_id: customerId,
      payment_method_id: paymentMethodId,
      price: priceCents,
      currency: 'usd',
      interval_unit: options.intervalUnit || 'month',
      interval_count: options.intervalCount || 1,
      ...(options.billingCycleAnchor && { billing_cycle_anchor: options.billingCycleAnchor }),
      ...(options.trialEnd && { trial_end: options.trialEnd }),
      ...(options.cancelAtPeriodEnd && { cancel_at_period_end: options.cancelAtPeriodEnd }),
      metadata: options.metadata || {}
    });
    return response.data;
  }

  async cancelSubscription(subscriptionId) {
    this.initializeSDK();
    const response = await this.subscriptionsApi.cancelSubscription(
      this.config.accountId,
      subscriptionId
    );
    return response.data;
  }

  async updateCustomer(customerId, updates) {
    this.initializeSDK();
    const response = await this.customersApi.updateCustomer(
      this.config.accountId,
      customerId,
      {
        ...(updates.email && { email: updates.email }),
        ...(updates.name && { first_name: updates.name }),
        ...(updates.metadata && { metadata: updates.metadata })
      }
    );
    return response.data;
  }

  async updateSubscription(subscriptionId, updates) {
    this.initializeSDK();
    const response = await this.subscriptionsApi.updateSubscription(
      this.config.accountId,
      subscriptionId,
      {
        ...(updates.paymentMethodId && { payment_method_id: updates.paymentMethodId }),
        ...(updates.metadata && { metadata: updates.metadata }),
        ...(typeof updates.cancel_at_period_end !== 'undefined' && {
          cancel_at_period_end: updates.cancel_at_period_end
        })
        // NOTE: Tilled does NOT allow changing billing cycles after creation
        // price changes and plan upgrades may be supported depending on Tilled's API
      }
    );
    return response.data;
  }

  verifyWebhookSignature(rawBody, signature, tolerance = 300) {
    if (!signature || !rawBody) return false;

    try {
      const parts = signature.split(',');
      const timestampPart = parts.find(p => p.startsWith('t='));
      const signaturePart = parts.find(p => p.startsWith('v1='));

      if (!timestampPart || !signaturePart) return false;

      const timestamp = timestampPart.split('=')[1];
      const receivedSignature = signaturePart.split('=')[1];

      // Fail-fast: Check timestamp tolerance BEFORE HMAC (prevent replay attacks)
      const currentTime = Math.floor(Date.now() / 1000);
      const webhookTime = parseInt(timestamp, 10);
      if (Math.abs(currentTime - webhookTime) > tolerance) return false;

      // Calculate expected signature
      const signedPayload = `${timestamp}.${rawBody}`;
      const expectedSignature = crypto
        .createHmac('sha256', this.config.webhookSecret)
        .update(signedPayload)
        .digest('hex');

      // Length check before timingSafeEqual (prevent crashes)
      if (expectedSignature.length !== receivedSignature.length) return false;

      return crypto.timingSafeEqual(
        Buffer.from(expectedSignature),
        Buffer.from(receivedSignature)
      );
    } catch (error) {
      console.error('Webhook signature verification error:', error);
      return false;
    }
  }

  /**
   * Create a one-time charge
   *
   * NOTE: This implementation uses PaymentIntentsApi as Tilled typically uses
   * payment intents for charges. If Tilled SDK has a dedicated ChargesApi,
   * adjust accordingly.
   *
   * @param {Object} params - Charge parameters
   * @param {string} params.appId - Application ID
   * @param {string} params.tilledCustomerId - Tilled customer ID
   * @param {string} params.paymentMethodId - Payment method ID
   * @param {number} params.amountCents - Amount in cents
   * @param {string} params.currency - Currency code (default: 'usd')
   * @param {string} params.description - Charge description
   * @param {Object} params.metadata - Additional metadata
   * @returns {Promise<Object>} Charge response with { id, status, failure_code, failure_message }
   */
  async createCharge({
    appId,
    tilledCustomerId,
    paymentMethodId,
    amountCents,
    currency = 'usd',
    description,
    metadata = {},
  }) {
    this.initializeSDK();

    try {
      const tilled = require('tilled-node');

      // Initialize PaymentIntentsApi if not already done
      if (!this.paymentIntentsApi) {
        const sdkConfig = new tilled.Configuration({
          apiKey: this.config.secretKey,
          basePath: this.config.basePath
        });
        this.paymentIntentsApi = new tilled.PaymentIntentsApi(sdkConfig);
      }

      // Create and confirm payment intent in one call
      const response = await this.paymentIntentsApi.createPaymentIntent(
        this.config.accountId,
        {
          amount: amountCents,
          currency,
          customer_id: tilledCustomerId,
          payment_method_id: paymentMethodId,
          description,
          metadata,
          confirm: true, // Auto-confirm the payment
          capture_method: 'automatic', // Capture immediately
        }
      );

      const paymentIntent = response.data;

      return {
        id: paymentIntent.id,
        status: paymentIntent.status === 'succeeded' ? 'succeeded' : 'pending',
        failure_code: paymentIntent.last_payment_error?.code || null,
        failure_message: paymentIntent.last_payment_error?.message || null,
      };
    } catch (error) {
      // Extract Tilled error details
      const errorCode = error.response?.data?.code || error.code || 'unknown';
      const errorMessage = error.response?.data?.message || error.message;

      throw Object.assign(new Error(errorMessage), {
        code: errorCode,
        message: errorMessage,
      });
    }
  }

  /**
   * Create a refund for a charge
   *
   * @param {Object} params - Refund parameters
   * @param {string} params.appId - Application ID
   * @param {string} params.tilledChargeId - Tilled charge/payment intent ID to refund
   * @param {number} params.amountCents - Amount in cents to refund
   * @param {string} params.currency - Currency code (default: 'usd')
   * @param {string} params.reason - Refund reason
   * @param {Object} params.metadata - Additional metadata
   * @returns {Promise<Object>} Refund response with { id, status, amount, currency, charge_id }
   */
  async createRefund({
    appId,
    tilledChargeId,
    amountCents,
    currency = 'usd',
    reason,
    metadata = {},
  }) {
    this.initializeSDK();

    try {
      const response = await this.refundsApi.createRefund(
        this.config.accountId,
        {
          payment_intent_id: tilledChargeId,
          amount: amountCents,
          currency,
          reason,
          metadata,
        }
      );

      const refund = response.data;

      return {
        id: refund.id,
        status: refund.status,
        amount: refund.amount,
        currency: refund.currency,
        charge_id: refund.payment_intent_id || refund.charge_id,
      };
    } catch (error) {
      // Extract Tilled error details
      const errorCode = error.response?.data?.code || error.code || 'unknown';
      const errorMessage = error.response?.data?.message || error.message;

      throw Object.assign(new Error(errorMessage), {
        code: errorCode,
        message: errorMessage,
      });
    }
  }

  /**
   * Get a refund by ID
   *
   * @param {string} refundId - Tilled refund ID
   * @returns {Promise<Object>} Refund details
   */
  async getRefund(refundId) {
    this.initializeSDK();

    try {
      const response = await this.refundsApi.getRefund(
        this.config.accountId,
        refundId
      );
      return response.data;
    } catch (error) {
      const errorCode = error.response?.data?.code || error.code || 'unknown';
      const errorMessage = error.response?.data?.message || error.message;

      throw Object.assign(new Error(errorMessage), {
        code: errorCode,
        message: errorMessage,
      });
    }
  }

  /**
   * List refunds with optional filters
   *
   * @param {Object} filters - Filter parameters
   * @returns {Promise<Array>} List of refunds
   */
  async listRefunds(filters = {}) {
    this.initializeSDK();

    try {
      const response = await this.refundsApi.listRefunds(
        this.config.accountId,
        filters
      );
      return response.data;
    } catch (error) {
      const errorCode = error.response?.data?.code || error.code || 'unknown';
      const errorMessage = error.response?.data?.message || error.message;

      throw Object.assign(new Error(errorMessage), {
        code: errorCode,
        message: errorMessage,
      });
    }
  }

  /**
   * Get a dispute by ID
   *
   * @param {string} disputeId - Tilled dispute ID
   * @returns {Promise<Object>} Dispute details
   */
  async getDispute(disputeId) {
    this.initializeSDK();

    try {
      const response = await this.disputesApi.getDispute(
        this.config.accountId,
        disputeId
      );
      return response.data;
    } catch (error) {
      const errorCode = error.response?.data?.code || error.code || 'unknown';
      const errorMessage = error.response?.data?.message || error.message;

      throw Object.assign(new Error(errorMessage), {
        code: errorCode,
        message: errorMessage,
      });
    }
  }

  /**
   * List disputes with optional filters
   *
   * @param {Object} filters - Filter parameters
   * @returns {Promise<Array>} List of disputes
   */
  async listDisputes(filters = {}) {
    this.initializeSDK();

    try {
      const response = await this.disputesApi.listDisputes(
        this.config.accountId,
        filters
      );
      return response.data;
    } catch (error) {
      const errorCode = error.response?.data?.code || error.code || 'unknown';
      const errorMessage = error.response?.data?.message || error.message;

      throw Object.assign(new Error(errorMessage), {
        code: errorCode,
        message: errorMessage,
      });
    }
  }
}

module.exports = TilledClient;
