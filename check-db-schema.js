// Check if app_id column exists in billing_refunds table
const { PrismaClient } = require('./node_modules/.prisma/ar');

async function checkSchema() {
  const prisma = new PrismaClient({
    datasources: {
      db: {
        url: process.env.DATABASE_URL_BILLING
      }
    }
  });

  try {
    const result = await prisma.$queryRaw`
      SELECT COLUMN_NAME, DATA_TYPE, IS_NULLABLE
      FROM INFORMATION_SCHEMA.COLUMNS
      WHERE TABLE_SCHEMA = 'billing_test'
        AND TABLE_NAME = 'billing_refunds'
      ORDER BY ORDINAL_POSITION
    `;
    console.log('billing_refunds columns:');
    result.forEach(col => {
      console.log(`  ${col.COLUMN_NAME}: ${col.DATA_TYPE} (nullable: ${col.IS_NULLABLE})`);
    });

    const hasAppId = result.some(col => col.COLUMN_NAME === 'app_id');
    console.log('\napp_id column exists:', hasAppId);
  } catch (error) {
    console.error('Error:', error.message);
  } finally {
    await prisma.$disconnect();
  }
}

checkSchema();
