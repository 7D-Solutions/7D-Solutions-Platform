// Check if billing_refunds table exists
const { PrismaClient } = require('./node_modules/.prisma/ar');

async function checkTables() {
  const prisma = new PrismaClient({
    datasources: {
      db: {
        url: process.env.DATABASE_URL_BILLING
      }
    }
  });

  try {
    const result = await prisma.$queryRaw`
      SELECT TABLE_NAME
      FROM INFORMATION_SCHEMA.TABLES
      WHERE TABLE_SCHEMA = 'billing_test'
      ORDER BY TABLE_NAME
    `;
    console.log('Tables in billing_test database:');
    result.forEach(table => {
      console.log(`  - ${table.TABLE_NAME}`);
    });

    const hasRefundsTable = result.some(table => table.TABLE_NAME === 'billing_refunds');
    console.log('\nbilling_refunds table exists:', hasRefundsTable);
  } catch (error) {
    console.error('Error:', error.message);
  } finally {
    await prisma.$disconnect();
  }
}

checkTables();
