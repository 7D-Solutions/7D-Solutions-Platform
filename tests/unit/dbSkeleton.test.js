const { billingPrisma } = require('../../backend/src/prisma');

describe('DB Skeleton Validation', () => {
  afterAll(async () => {
    // Clean up test data
    await billingPrisma.billing_events.deleteMany({
      where: { app_id: 'test_skeleton' }
    });
    await billingPrisma.$disconnect();
  });

  it('validates billing_events table exists and is writable', async () => {
    // Insert a test event
    const event = await billingPrisma.billing_events.create({
      data: {
        app_id: 'test_skeleton',
        event_type: 'test.skeleton.validation',
        source: 'system',
        entity_type: 'test',
        entity_id: '123',
        payload: { test: true }
      }
    });

    expect(event.id).toBeDefined();
    expect(event.app_id).toBe('test_skeleton');
    expect(event.event_type).toBe('test.skeleton.validation');
    expect(event.source).toBe('system');

    // Read it back
    const retrieved = await billingPrisma.billing_events.findUnique({
      where: { id: event.id }
    });

    expect(retrieved).not.toBeNull();
    expect(retrieved.payload).toEqual({ test: true });
  });

  it('validates billing_idempotency_keys table exists and is writable', async () => {
    const expires = new Date(Date.now() + 86400000); // 24 hours

    const key = await billingPrisma.billing_idempotency_keys.create({
      data: {
        app_id: 'test_skeleton',
        idempotency_key: 'test-key-123',
        request_hash: 'abc123def456',
        response_body: { status: 'success' },
        status_code: 201,
        expires_at: expires
      }
    });

    expect(key.id).toBeDefined();
    expect(key.idempotency_key).toBe('test-key-123');

    // Clean up
    await billingPrisma.billing_idempotency_keys.delete({
      where: { id: key.id }
    });
  });

  it('validates billing_plans table exists and is writable', async () => {
    const plan = await billingPrisma.billing_plans.create({
      data: {
        app_id: 'test_skeleton',
        plan_id: 'test-plan',
        name: 'Test Plan',
        interval_unit: 'month',
        interval_count: 1,
        price_cents: 9900,
        features: { test: true }
      }
    });

    expect(plan.id).toBeDefined();
    expect(plan.plan_id).toBe('test-plan');
    expect(plan.features).toEqual({ test: true });

    // Clean up
    await billingPrisma.billing_plans.delete({
      where: { id: plan.id }
    });
  });

  it('validates all Phase 2-4 tables exist in Prisma client', () => {
    // Phase 2: Reliability & Safety
    expect(billingPrisma.billing_idempotency_keys).toBeDefined();
    expect(billingPrisma.billing_events).toBeDefined();
    expect(billingPrisma.billing_webhook_attempts).toBeDefined();
    expect(billingPrisma.billing_reconciliation_runs).toBeDefined();
    expect(billingPrisma.billing_divergences).toBeDefined();

    // Phase 3: Pricing Agility
    expect(billingPrisma.billing_plans).toBeDefined();
    expect(billingPrisma.billing_coupons).toBeDefined();
    expect(billingPrisma.billing_addons).toBeDefined();
    expect(billingPrisma.billing_subscription_addons).toBeDefined();

    // Phase 4: Money Records
    expect(billingPrisma.billing_invoices).toBeDefined();
    expect(billingPrisma.billing_charges).toBeDefined();
    expect(billingPrisma.billing_refunds).toBeDefined();
    expect(billingPrisma.billing_disputes).toBeDefined();
  });
});
