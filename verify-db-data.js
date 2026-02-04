// Verify database contains refund/dispute data
const { PrismaClient } = require('./node_modules/.prisma/ar');

async function verifyData() {
  const dbUrl = process.env.DATABASE_URL_BILLING || 'mysql://billing_test:testpass@localhost:3309/billing_test';
  console.log('Connecting to:', dbUrl.replace(/:([^:@]+)@/, ':***@'));

  const prisma = new PrismaClient({
    datasources: {
      db: {
        url: dbUrl
      }
    }
  });

  try {
    const dbCheck = await prisma.$queryRaw`SELECT DATABASE() as db`;
    console.log('Connected to database:', dbCheck[0].db);
    console.log('');
    console.log('=== REFUNDS ===');
    const refunds = await prisma.$queryRaw`
      SELECT id, app_id, status, tilled_refund_id, reference_id, amount_cents, created_at
      FROM billing_refunds
      ORDER BY id DESC
      LIMIT 5
    `;
    console.table(refunds);

    console.log('\n=== DISPUTES ===');
    const disputes = await prisma.$queryRaw`
      SELECT id, app_id, status, tilled_dispute_id, amount_cents, created_at
      FROM billing_disputes
      ORDER BY id DESC
      LIMIT 5
    `;
    console.table(disputes);

    console.log('\n=== WEBHOOK EVENTS ===');
    const webhooks = await prisma.$queryRaw`
      SELECT event_id, event_type, COUNT(*) as count
      FROM billing_webhooks
      GROUP BY event_id, event_type
      HAVING COUNT(*) > 1
    `;
    console.log('Duplicate webhook events (should be empty):');
    console.table(webhooks);

    console.log('\n=== WEBHOOK EVENT UNIQUENESS ===');
    const allWebhooks = await prisma.$queryRaw`
      SELECT event_id, COUNT(*) as delivery_count
      FROM billing_webhooks
      GROUP BY event_id
      ORDER BY delivery_count DESC
      LIMIT 10
    `;
    console.table(allWebhooks);

  } catch (error) {
    console.error('Error:', error.message);
  } finally {
    await prisma.$disconnect();
  }
}

verifyData();
