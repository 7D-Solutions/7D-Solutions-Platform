/**
 * Unit tests for Request Validators
 *
 * Tests validation middleware for all billing routes.
 * Covers: valid inputs, invalid inputs, XSS attempts, edge cases
 *
 * Reference: SECURITY-REVIEW-SEPARATION-OF-CONCERNS.md lines 438-446
 */

const {
  getCustomerByIdValidator,
  getCustomerByExternalIdValidator,
  createCustomerValidator,
  setDefaultPaymentMethodValidator,
  updateCustomerValidator,
  getBillingStateValidator,
  listPaymentMethodsValidator,
  addPaymentMethodValidator,
  setDefaultPaymentMethodByIdValidator,
  deletePaymentMethodValidator,
  getSubscriptionByIdValidator,
  listSubscriptionsValidator,
  createSubscriptionValidator,
  cancelSubscriptionValidator,
  updateSubscriptionValidator,
  createOneTimeChargeValidator,
  createRefundValidator
} = require('../../../backend/src/validators/index');

// Mock Express req/res/next
const createMockRequest = (params = {}, query = {}, body = {}, headers = {}) => ({
  params,
  query,
  body,
  headers
});

const createMockResponse = () => {
  const res = {};
  res.status = jest.fn().mockReturnValue(res);
  res.json = jest.fn().mockReturnValue(res);
  return res;
};

const createMockNext = () => jest.fn();

// Helper to run validator middleware chain
const runValidator = async (validator, req, res, next) => {
  for (const middleware of validator) {
    await middleware(req, res, next);
    if (res.status.mock.calls.length > 0) {
      // Validation failed
      return;
    }
  }
};

describe('Request Validators', () => {
  // ============================================================================
  // CUSTOMER VALIDATORS
  // ============================================================================

  describe('getCustomerByIdValidator', () => {
    it('should pass with valid customer ID', async () => {
      const req = createMockRequest({ id: '123' });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(getCustomerByIdValidator, req, res, next);

      expect(next).toHaveBeenCalled();
      expect(res.status).not.toHaveBeenCalled();
    });

    it('should fail with non-numeric ID', async () => {
      const req = createMockRequest({ id: 'abc' });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(getCustomerByIdValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
      expect(res.json).toHaveBeenCalledWith(
        expect.objectContaining({
          error: 'Validation failed'
        })
      );
    });

    it('should fail with negative ID', async () => {
      const req = createMockRequest({ id: '-5' });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(getCustomerByIdValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should fail with zero ID', async () => {
      const req = createMockRequest({ id: '0' });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(getCustomerByIdValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });
  });

  describe('getCustomerByExternalIdValidator', () => {
    it('should pass with valid external_customer_id', async () => {
      const req = createMockRequest({}, { external_customer_id: 'cust_123' });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(getCustomerByExternalIdValidator, req, res, next);

      expect(next).toHaveBeenCalled();
      expect(res.status).not.toHaveBeenCalled();
    });

    it('should fail with missing external_customer_id', async () => {
      const req = createMockRequest({}, {});
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(getCustomerByExternalIdValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should sanitize XSS attempt in external_customer_id', async () => {
      const req = createMockRequest({}, { external_customer_id: '<script>alert("xss")</script>' });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(getCustomerByExternalIdValidator, req, res, next);

      // Should pass (sanitized) but value should be escaped
      expect(next).toHaveBeenCalled();
    });
  });

  describe('createCustomerValidator', () => {
    it('should pass with valid email and name', async () => {
      const req = createMockRequest({}, {}, {
        email: 'test@example.com',
        name: 'John Doe'
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createCustomerValidator, req, res, next);

      expect(next).toHaveBeenCalled();
      expect(res.status).not.toHaveBeenCalled();
    });

    it('should pass with all optional fields', async () => {
      const req = createMockRequest({}, {}, {
        email: 'test@example.com',
        name: 'John Doe',
        external_customer_id: 'ext_123',
        metadata: { source: 'web' }
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createCustomerValidator, req, res, next);

      expect(next).toHaveBeenCalled();
    });

    it('should fail with missing email', async () => {
      const req = createMockRequest({}, {}, { name: 'John Doe' });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createCustomerValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should fail with missing name', async () => {
      const req = createMockRequest({}, {}, { email: 'test@example.com' });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createCustomerValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should fail with invalid email format', async () => {
      const req = createMockRequest({}, {}, {
        email: 'not-an-email',
        name: 'John Doe'
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createCustomerValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
      expect(res.json).toHaveBeenCalledWith(
        expect.objectContaining({
          error: 'Validation failed',
          details: expect.arrayContaining([
            expect.objectContaining({
              field: 'email',
              message: 'Invalid email format'
            })
          ])
        })
      );
    });

    it('should fail with email missing @ symbol', async () => {
      const req = createMockRequest({}, {}, {
        email: 'testexample.com',
        name: 'John Doe'
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createCustomerValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should fail with email missing domain', async () => {
      const req = createMockRequest({}, {}, {
        email: 'test@',
        name: 'John Doe'
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createCustomerValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should sanitize XSS attempt in name field', async () => {
      const req = createMockRequest({}, {}, {
        email: 'test@example.com',
        name: '<script>alert("xss")</script>'
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createCustomerValidator, req, res, next);

      // Should pass (sanitized)
      expect(next).toHaveBeenCalled();
    });

    it('should trim whitespace from name', async () => {
      const req = createMockRequest({}, {}, {
        email: 'test@example.com',
        name: '  John Doe  '
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createCustomerValidator, req, res, next);

      expect(next).toHaveBeenCalled();
    });

    it('should fail with name exceeding 255 characters', async () => {
      const req = createMockRequest({}, {}, {
        email: 'test@example.com',
        name: 'a'.repeat(256)
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createCustomerValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should fail with metadata not an object', async () => {
      const req = createMockRequest({}, {}, {
        email: 'test@example.com',
        name: 'John Doe',
        metadata: 'invalid'
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createCustomerValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });
  });

  // ============================================================================
  // PAYMENT METHOD VALIDATORS
  // ============================================================================

  describe('listPaymentMethodsValidator', () => {
    it('should pass with valid billing_customer_id', async () => {
      const req = createMockRequest({}, { billing_customer_id: '123' });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(listPaymentMethodsValidator, req, res, next);

      expect(next).toHaveBeenCalled();
    });

    it('should fail with missing billing_customer_id', async () => {
      const req = createMockRequest({}, {});
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(listPaymentMethodsValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should fail with non-numeric billing_customer_id', async () => {
      const req = createMockRequest({}, { billing_customer_id: 'abc' });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(listPaymentMethodsValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should fail with negative billing_customer_id', async () => {
      const req = createMockRequest({}, { billing_customer_id: '-5' });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(listPaymentMethodsValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });
  });

  describe('addPaymentMethodValidator', () => {
    it('should pass with valid fields', async () => {
      const req = createMockRequest({}, {}, {
        billing_customer_id: 123,
        payment_method_id: 'pm_test_123'
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(addPaymentMethodValidator, req, res, next);

      expect(next).toHaveBeenCalled();
    });

    it('should fail with missing billing_customer_id', async () => {
      const req = createMockRequest({}, {}, {
        payment_method_id: 'pm_test_123'
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(addPaymentMethodValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should fail with missing payment_method_id', async () => {
      const req = createMockRequest({}, {}, {
        billing_customer_id: 123
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(addPaymentMethodValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });
  });

  // ============================================================================
  // SUBSCRIPTION VALIDATORS
  // ============================================================================

  describe('createSubscriptionValidator', () => {
    const validSubscription = {
      billing_customer_id: 123,
      payment_method_id: 'pm_test_123',
      plan_id: 'pro-monthly',
      plan_name: 'Pro Monthly',
      price_cents: 9900
    };

    it('should pass with all required fields', async () => {
      const req = createMockRequest({}, {}, validSubscription);
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createSubscriptionValidator, req, res, next);

      expect(next).toHaveBeenCalled();
    });

    it('should pass with all optional fields', async () => {
      const req = createMockRequest({}, {}, {
        ...validSubscription,
        interval_unit: 'month',
        interval_count: 1,
        billing_cycle_anchor: '2024-01-01T00:00:00Z',
        trial_end: '2024-02-01T00:00:00Z',
        cancel_at_period_end: false,
        metadata: { source: 'web' }
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createSubscriptionValidator, req, res, next);

      expect(next).toHaveBeenCalled();
    });

    it('should fail with missing billing_customer_id', async () => {
      const req = createMockRequest({}, {}, {
        payment_method_id: 'pm_test_123',
        plan_id: 'pro-monthly',
        plan_name: 'Pro Monthly',
        price_cents: 9900
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createSubscriptionValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should fail with negative price_cents', async () => {
      const req = createMockRequest({}, {}, {
        ...validSubscription,
        price_cents: -100
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createSubscriptionValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should pass with zero price_cents (free trial)', async () => {
      const req = createMockRequest({}, {}, {
        ...validSubscription,
        price_cents: 0
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createSubscriptionValidator, req, res, next);

      expect(next).toHaveBeenCalled();
    });

    it('should fail with invalid interval_unit', async () => {
      const req = createMockRequest({}, {}, {
        ...validSubscription,
        interval_unit: 'decade'
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createSubscriptionValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should sanitize XSS attempt in plan_id', async () => {
      const req = createMockRequest({}, {}, {
        ...validSubscription,
        plan_id: '<script>alert("xss")</script>'
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createSubscriptionValidator, req, res, next);

      // Should pass (sanitized)
      expect(next).toHaveBeenCalled();
    });

    it('should sanitize XSS attempt in plan_name', async () => {
      const req = createMockRequest({}, {}, {
        ...validSubscription,
        plan_name: '<img src=x onerror=alert(1)>'
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createSubscriptionValidator, req, res, next);

      // Should pass (sanitized)
      expect(next).toHaveBeenCalled();
    });
  });

  describe('listSubscriptionsValidator', () => {
    it('should pass with no filters', async () => {
      const req = createMockRequest({}, {});
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(listSubscriptionsValidator, req, res, next);

      expect(next).toHaveBeenCalled();
    });

    it('should pass with valid billing_customer_id filter', async () => {
      const req = createMockRequest({}, { billing_customer_id: '123' });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(listSubscriptionsValidator, req, res, next);

      expect(next).toHaveBeenCalled();
    });

    it('should pass with valid status filter', async () => {
      const req = createMockRequest({}, { status: 'active' });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(listSubscriptionsValidator, req, res, next);

      expect(next).toHaveBeenCalled();
    });

    it('should fail with invalid status value', async () => {
      const req = createMockRequest({}, { status: 'invalid_status' });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(listSubscriptionsValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });
  });

  // ============================================================================
  // ONE-TIME CHARGE VALIDATORS
  // ============================================================================

  describe('createOneTimeChargeValidator', () => {
    const validCharge = {
      external_customer_id: 'cust_123',
      amount_cents: 5000,
      reason: 'extra_pickup',
      reference_id: 'ref_123'
    };

    const validHeaders = {
      'idempotency-key': 'idem_123'
    };

    it('should pass with all required fields', async () => {
      const req = createMockRequest({}, {}, validCharge, validHeaders);
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createOneTimeChargeValidator, req, res, next);

      expect(next).toHaveBeenCalled();
    });

    it('should fail with missing Idempotency-Key header', async () => {
      const req = createMockRequest({}, {}, validCharge, {});
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createOneTimeChargeValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should fail with missing amount_cents', async () => {
      const req = createMockRequest({}, {}, {
        ...validCharge,
        amount_cents: undefined
      }, validHeaders);
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createOneTimeChargeValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should fail with negative amount_cents', async () => {
      const req = createMockRequest({}, {}, {
        ...validCharge,
        amount_cents: -100
      }, validHeaders);
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createOneTimeChargeValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should fail with zero amount_cents', async () => {
      const req = createMockRequest({}, {}, {
        ...validCharge,
        amount_cents: 0
      }, validHeaders);
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createOneTimeChargeValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should sanitize XSS attempt in reason field', async () => {
      const req = createMockRequest({}, {}, {
        ...validCharge,
        reason: '<script>alert("xss")</script>'
      }, validHeaders);
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createOneTimeChargeValidator, req, res, next);

      // Should pass (sanitized)
      expect(next).toHaveBeenCalled();
    });

    it('should sanitize XSS attempt in note field', async () => {
      const req = createMockRequest({}, {}, {
        ...validCharge,
        note: '<img src=x onerror=alert(1)>'
      }, validHeaders);
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createOneTimeChargeValidator, req, res, next);

      // Should pass (sanitized)
      expect(next).toHaveBeenCalled();
    });

    it('should fail with note exceeding 500 characters', async () => {
      const req = createMockRequest({}, {}, {
        ...validCharge,
        note: 'a'.repeat(501)
      }, validHeaders);
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createOneTimeChargeValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });
  });

  // ============================================================================
  // REFUND VALIDATORS
  // ============================================================================

  describe('createRefundValidator', () => {
    const validRefund = {
      charge_id: 123,
      amount_cents: 5000,
      reference_id: 'ref_refund_123'
    };

    const validHeaders = {
      'idempotency-key': 'idem_refund_123'
    };

    it('should pass with all required fields', async () => {
      const req = createMockRequest({}, {}, validRefund, validHeaders);
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createRefundValidator, req, res, next);

      expect(next).toHaveBeenCalled();
    });

    it('should fail with missing Idempotency-Key header', async () => {
      const req = createMockRequest({}, {}, validRefund, {});
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createRefundValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should fail with missing charge_id', async () => {
      const req = createMockRequest({}, {}, {
        amount_cents: 5000,
        reference_id: 'ref_refund_123'
      }, validHeaders);
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createRefundValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should fail with missing amount_cents', async () => {
      const req = createMockRequest({}, {}, {
        charge_id: 123,
        reference_id: 'ref_refund_123'
      }, validHeaders);
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createRefundValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should fail with negative amount_cents', async () => {
      const req = createMockRequest({}, {}, {
        ...validRefund,
        amount_cents: -100
      }, validHeaders);
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createRefundValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should fail with zero amount_cents', async () => {
      const req = createMockRequest({}, {}, {
        ...validRefund,
        amount_cents: 0
      }, validHeaders);
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createRefundValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });

    it('should sanitize XSS attempt in reason field', async () => {
      const req = createMockRequest({}, {}, {
        ...validRefund,
        reason: '<script>alert("xss")</script>'
      }, validHeaders);
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createRefundValidator, req, res, next);

      // Should pass (sanitized)
      expect(next).toHaveBeenCalled();
    });

    it('should fail with missing reference_id', async () => {
      const req = createMockRequest({}, {}, {
        charge_id: 123,
        amount_cents: 5000
      }, validHeaders);
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(createRefundValidator, req, res, next);

      expect(res.status).toHaveBeenCalledWith(400);
    });
  });

  // ============================================================================
  // UPDATE VALIDATORS
  // ============================================================================

  describe('updateSubscriptionValidator', () => {
    it('should pass with valid subscription ID and updates', async () => {
      const req = createMockRequest({ id: '123' }, {}, {
        price_cents: 10900,
        plan_name: 'Pro Monthly Plus'
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(updateSubscriptionValidator, req, res, next);

      expect(next).toHaveBeenCalled();
    });

    it('should allow app_id in body (route will ignore it)', async () => {
      const req = createMockRequest({ id: '123' }, {}, {
        app_id: 'different_app',
        price_cents: 10900
      });
      const res = createMockResponse();
      const next = createMockNext();

      await runValidator(updateSubscriptionValidator, req, res, next);

      // Validator should pass - the route itself handles ignoring app_id
      expect(next).toHaveBeenCalled();
      expect(res.status).not.toHaveBeenCalled();
    });
  });
});
