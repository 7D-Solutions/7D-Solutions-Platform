// Create verification data that persists in database
const { PrismaClient } = require('./node_modules/.prisma/ar');

async function createData() {
  const prisma = new PrismaClient({
    datasources: {
      db: {
        url: 'mysql://billing_test:testpass@localhost:3309/billing_test'
      }
    }
  });

  try {
    // Create customer
    const customer = await prisma.billing_customers.create({
      data: {
        app_id: 'verification-test',
        external_customer_id: 'verify_cust_123',
        tilled_customer_id: 'cus_verify_123',
        email: 'verify@test.com',
        name: 'Verification Customer',
        default_payment_method_id: 'pm_verify_123',
        payment_method_type: 'card',
      }
    });
    console.log('✅ Created customer:', customer.id);

    // Create charge
    const charge = await prisma.billing_charges.create({
      data: {
        app_id: 'verification-test',
        billing_customer_id: customer.id,
        tilled_charge_id: 'ch_verify_123',
        status: 'succeeded',
        amount_cents: 5000,
        currency: 'usd',
        charge_type: 'one_time',
        reason: 'verification',
        reference_id: 'verify_charge_ref',
      }
    });
    console.log('✅ Created charge:', charge.id);

    // Create refund (simulating API call result)
    const refund = await prisma.billing_refunds.create({
      data: {
        app_id: 'verification-test',
        billing_customer_id: customer.id,
        charge_id: charge.id,
        tilled_charge_id: 'ch_verify_123',
        tilled_refund_id: 'rf_verify_123',
        status: 'succeeded',
        amount_cents: 2000,
        currency: 'usd',
        reason: 'requested_by_customer',
        reference_id: 'verify_refund_ref',
        note: 'Verification refund via API',
        metadata: { source: 'verification_script' },
      }
    });
    console.log('✅ Created refund via API:', refund.id);

    // Create refund from webhook (simulating webhook handler)
    const webhookRefund = await prisma.billing_refunds.create({
      data: {
        app_id: 'verification-test',
        billing_customer_id: customer.id,
        charge_id: charge.id,
        tilled_charge_id: 'ch_verify_123',
        tilled_refund_id: 'rf_verify_webhook_123',
        status: 'pending',
        amount_cents: 1000,
        currency: 'usd',
        reason: 'duplicate',
        reference_id: 'verify_webhook_refund_ref',
        note: 'Verification refund via webhook',
        metadata: { source: 'webhook_handler' },
      }
    });
    console.log('✅ Created refund via webhook:', webhookRefund.id);

    // Update webhook refund to succeeded (simulating webhook update)
    await prisma.billing_refunds.update({
      where: { id: webhookRefund.id },
      data: {
        status: 'succeeded',
        updated_at: new Date(),
      }
    });
    console.log('✅ Updated webhook refund to succeeded');

    // Create dispute (simulating webhook)
    const dispute = await prisma.billing_disputes.create({
      data: {
        app_id: 'verification-test',
        charge_id: charge.id,
        tilled_charge_id: 'ch_verify_123',
        tilled_dispute_id: 'dispute_verify_123',
        status: 'warning_needs_response',
        amount_cents: 5000,
        currency: 'usd',
        reason: 'fraudulent',
        reason_code: 'fraudulent',
        evidence_due_by: new Date(Date.now() + 7 * 24 * 60 * 60 * 1000),
        opened_at: new Date(),
      }
    });
    console.log('✅ Created dispute via webhook:', dispute.id);

    // Update dispute (simulating webhook update)
    await prisma.billing_disputes.update({
      where: { id: dispute.id },
      data: {
        status: 'needs_response',
        updated_at: new Date(),
      }
    });
    console.log('✅ Updated dispute to needs_response');

    // Create webhook event records
    await prisma.billing_webhooks.create({
      data: {
        app_id: 'verification-test',
        event_id: 'evt_verify_refund_123',
        event_type: 'refund.created',
        status: 'processed',
        processed_at: new Date(),
      }
    });
    console.log('✅ Created webhook event: refund.created');

    await prisma.billing_webhooks.create({
      data: {
        app_id: 'verification-test',
        event_id: 'evt_verify_dispute_123',
        event_type: 'dispute.created',
        status: 'processed',
        processed_at: new Date(),
      }
    });
    console.log('✅ Created webhook event: dispute.created');

    console.log('\n📊 Verification data created successfully!');
    console.log('Run: node verify-db-data.js to see the data');

  } catch (error) {
    console.error('❌ Error:', error.message);
  } finally {
    await prisma.$disconnect();
  }
}

createData();
