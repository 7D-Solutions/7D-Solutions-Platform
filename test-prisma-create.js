// Test creating a refund with app_id
const { PrismaClient } = require('./node_modules/.prisma/ar');

async function test() {
  const client = new PrismaClient({
    datasources: {
      db: {
        url: process.env.DATABASE_URL_BILLING
      }
    }
  });

  try {
    console.log('Attempting to create refund with app_id...');
    const result = await client.billing_refunds.create({
      data: {
        app_id: 'test-app',
        billing_customer_id: 1,
        charge_id: 1,
        tilled_charge_id: 'ch_test',
        status: 'pending',
        amount_cents: 1000,
        currency: 'usd',
        reference_id: 'test-ref-123',
      }
    });
    console.log('Success:', result);
  } catch (error) {
    console.log('Error:', error.message);
  } finally {
    await client.$disconnect();
  }
}

test();
