const TilledClient = require('./tilledClient');
const CustomerService = require('./services/CustomerService');
const PaymentMethodService = require('./services/PaymentMethodService');
const SubscriptionService = require('./services/SubscriptionService');
const ChargeService = require('./services/ChargeService');
const RefundService = require('./services/RefundService');
const WebhookService = require('./services/WebhookService');
const BillingStateService = require('./services/BillingStateService');
const IdempotencyService = require('./services/IdempotencyService');
const TaxService = require('./services/TaxService');
const DiscountService = require('./services/DiscountService');
const ProrationService = require('./services/ProrationService');
const UsageService = require('./services/UsageService');

/**
 * BillingService - Facade for all billing operations
 *
 * This class serves as a facade that delegates to specialized service modules:
 * - CustomerService: Customer CRUD operations
 * - PaymentMethodService: Payment method management
 * - SubscriptionService: Subscription lifecycle
 * - ChargeService: One-time charges
 * - RefundService: Refund management
 * - WebhookService: Webhook processing
 * - BillingStateService: Billing state aggregation
 * - IdempotencyService: Request idempotency
 * - TaxService: Tax calculation and management (Phase 1)
 * - DiscountService: Discount and promotion engine (Phase 2)
 * - ProrationService: Mid-cycle billing change calculations (Phase 3)
 * - UsageService: Metered usage billing and reporting (Phase 4)
 */
class BillingService {
  constructor() {
    this.tilledClients = new Map();

    // Bind getTilledClient to this so it can be passed to services
    this.getTilledClient = this.getTilledClient.bind(this);

    // Initialize specialized services
    this.customerService = new CustomerService(this.getTilledClient);
    this.paymentMethodService = new PaymentMethodService(this.getTilledClient, this.customerService);
    this.subscriptionService = new SubscriptionService(this.getTilledClient);
    this.chargeService = new ChargeService(this.getTilledClient);
    this.refundService = new RefundService(this.getTilledClient);
    this.webhookService = new WebhookService(this.getTilledClient);
    this.billingStateService = new BillingStateService();
    this.idempotencyService = new IdempotencyService();
    this.taxService = new TaxService();
    this.discountService = new DiscountService();
    this.prorationService = new ProrationService(this.getTilledClient);
    this.usageService = new UsageService(this.getTilledClient);
  }

  getTilledClient(appId) {
    if (!this.tilledClients.has(appId)) {
      this.tilledClients.set(appId, new TilledClient(appId));
    }
    return this.tilledClients.get(appId);
  }

  // ===========================================================
  // CUSTOMER SERVICE DELEGATION
  // ===========================================================

  async createCustomer(appId, email, name, externalCustomerId = null, metadata = {}) {
    return this.customerService.createCustomer(appId, email, name, externalCustomerId, metadata);
  }

  async getCustomerById(appId, billingCustomerId) {
    return this.customerService.getCustomerById(appId, billingCustomerId);
  }

  async findCustomer(appId, externalCustomerId) {
    return this.customerService.findCustomer(appId, externalCustomerId);
  }

  async updateCustomer(appId, billingCustomerId, patch) {
    return this.customerService.updateCustomer(appId, billingCustomerId, patch);
  }

  // ===========================================================
  // PAYMENT METHOD SERVICE DELEGATION
  // ===========================================================

  async setDefaultPaymentMethod(appId, customerId, paymentMethodId, paymentMethodType) {
    return this.paymentMethodService.setDefaultPaymentMethod(appId, customerId, paymentMethodId, paymentMethodType);
  }

  async listPaymentMethods(appId, billingCustomerId) {
    return this.paymentMethodService.listPaymentMethods(appId, billingCustomerId);
  }

  async addPaymentMethod(appId, billingCustomerId, paymentMethodId) {
    return this.paymentMethodService.addPaymentMethod(appId, billingCustomerId, paymentMethodId);
  }

  async setDefaultPaymentMethodById(appId, billingCustomerId, tilledPaymentMethodId) {
    return this.paymentMethodService.setDefaultPaymentMethodById(appId, billingCustomerId, tilledPaymentMethodId);
  }

  async deletePaymentMethod(appId, billingCustomerId, tilledPaymentMethodId) {
    return this.paymentMethodService.deletePaymentMethod(appId, billingCustomerId, tilledPaymentMethodId);
  }

  // ===========================================================
  // SUBSCRIPTION SERVICE DELEGATION
  // ===========================================================

  async createSubscription(appId, billingCustomerId, paymentMethodId, planId, planName, priceCents, options = {}) {
    return this.subscriptionService.createSubscription(appId, billingCustomerId, paymentMethodId, planId, planName, priceCents, options);
  }

  async cancelSubscription(subscriptionId) {
    return this.subscriptionService.cancelSubscription(subscriptionId);
  }

  async cancelSubscriptionEx(appId, subscriptionId, options = {}) {
    return this.subscriptionService.cancelSubscriptionEx(appId, subscriptionId, options);
  }

  async changeCycle(appId, payload) {
    return this.subscriptionService.changeCycle(appId, payload);
  }

  async getSubscriptionById(appId, subscriptionId) {
    return this.subscriptionService.getSubscriptionById(appId, subscriptionId);
  }

  async listSubscriptions(filters = {}) {
    return this.subscriptionService.listSubscriptions(filters);
  }

  async updateSubscription(appId, subscriptionId, patch) {
    return this.subscriptionService.updateSubscription(appId, subscriptionId, patch);
  }

  // ===========================================================
  // CHARGE SERVICE DELEGATION
  // ===========================================================

  async createOneTimeCharge(appId, chargeData, idempotencyData) {
    return this.chargeService.createOneTimeCharge(appId, chargeData, idempotencyData);
  }

  // ===========================================================
  // REFUND SERVICE DELEGATION
  // ===========================================================

  async createRefund(appId, refundData, idempotencyData) {
    return this.refundService.createRefund(appId, refundData, idempotencyData);
  }

  async getRefund(appId, refundId) {
    return this.refundService.getRefund(appId, refundId);
  }

  async listRefunds(appId, filters = {}) {
    return this.refundService.listRefunds(appId, filters);
  }

  // ===========================================================
  // WEBHOOK SERVICE DELEGATION
  // ===========================================================

  async processWebhook(appId, event, rawBody, signature) {
    return this.webhookService.processWebhook(appId, event, rawBody, signature);
  }

  async handleWebhookEvent(appId, event) {
    return this.webhookService.handleWebhookEvent(appId, event);
  }

  async handlePaymentFailure(paymentObject, eventType) {
    return this.webhookService.handlePaymentFailure(paymentObject, eventType);
  }

  async handleSubscriptionUpdate(tilledSubscription) {
    return this.webhookService.handleSubscriptionUpdate(tilledSubscription);
  }

  async handleSubscriptionCanceled(tilledSubscription) {
    return this.webhookService.handleSubscriptionCanceled(tilledSubscription);
  }

  // ===========================================================
  // BILLING STATE SERVICE DELEGATION
  // ===========================================================

  async getBillingState(appId, externalCustomerId) {
    return this.billingStateService.getBillingState(appId, externalCustomerId);
  }

  getEntitlements(appId, subscription) {
    return this.billingStateService.getEntitlements(appId, subscription);
  }

  // ===========================================================
  // IDEMPOTENCY SERVICE DELEGATION
  // ===========================================================

  computeRequestHash(method, path, body) {
    return this.idempotencyService.computeRequestHash(method, path, body);
  }

  async getIdempotentResponse(appId, idempotencyKey, requestHash) {
    return this.idempotencyService.getIdempotentResponse(appId, idempotencyKey, requestHash);
  }

  async storeIdempotentResponse(appId, idempotencyKey, requestHash, statusCode, responseBody, ttlDays = 30) {
    return this.idempotencyService.storeIdempotentResponse(appId, idempotencyKey, requestHash, statusCode, responseBody, ttlDays);
  }
  // ===========================================================
  // TAX SERVICE DELEGATION (PHASE 1)
  // ===========================================================

  async calculateTax(appId, customerId, subtotalCents, options = {}) {
    return this.taxService.calculateTax(appId, customerId, subtotalCents, options);
  }

  async getTaxRatesByJurisdiction(appId, jurisdictionCode) {
    return this.taxService.getTaxRatesByJurisdiction(appId, jurisdictionCode);
  }

  async createTaxRate(appId, jurisdictionCode, taxType, rate, options = {}) {
    return this.taxService.createTaxRate(appId, jurisdictionCode, taxType, rate, options);
  }

  async createTaxExemption(appId, customerId, taxType, certificateNumber) {
    return this.taxService.createTaxExemption(appId, customerId, taxType, certificateNumber);
  }

  async recordTaxCalculation(appId, taxRateId, taxableAmountCents, taxAmountCents, options = {}) {
    return this.taxService.recordTaxCalculation(appId, taxRateId, taxableAmountCents, taxAmountCents, options);
  }

  async getTaxCalculationsForInvoice(appId, invoiceId) {
    return this.taxService.getTaxCalculationsForInvoice(appId, invoiceId);
  }

  async getTaxCalculationsForCharge(appId, chargeId) {
    return this.taxService.getTaxCalculationsForCharge(appId, chargeId);
  }

  // ===========================================================
  // DISCOUNT SERVICE DELEGATION (PHASE 2)
  // ===========================================================

  async calculateDiscounts(appId, customerId, subtotalCents, options = {}) {
    return this.discountService.calculateDiscounts(appId, customerId, subtotalCents, options);
  }

  async applyDiscounts(appId, customerId, subtotalCents, couponCodes = []) {
    return this.discountService.calculateDiscounts(appId, customerId, subtotalCents, { couponCodes });
  }

  async validateCoupon(appId, couponCode, context = {}) {
    return this.discountService.validateCoupon(appId, couponCode, context);
  }

  async recordDiscount(appId, discountDetails) {
    return this.discountService.recordDiscount(appId, discountDetails);
  }

  async getDiscountsForInvoice(appId, invoiceId) {
    return this.discountService.getDiscountsForInvoice(appId, invoiceId);
  }

  async getAvailableDiscounts(appId, customerId, context = {}) {
    return this.discountService.getAvailableDiscounts(appId, customerId, context);
  }

  // ===========================================================
  // PRORATION SERVICE DELEGATION (PHASE 3)
  // ===========================================================

  async calculateProration(params) {
    return this.prorationService.calculateProration(params);
  }

  async applySubscriptionChange(subscriptionId, changeDetails, options = {}) {
    return this.prorationService.applySubscriptionChange(subscriptionId, changeDetails, options);
  }

  async calculateCancellationRefund(subscriptionId, cancellationDate, refundBehavior = 'partial_refund') {
    return this.prorationService.calculateCancellationRefund(subscriptionId, cancellationDate, refundBehavior);
  }
}

module.exports = BillingService;
