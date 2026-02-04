// Test database connection
const { PrismaClient } = require('./node_modules/.prisma/ar');

async function testConnection() {
  console.log('DATABASE_URL_BILLING:', process.env.DATABASE_URL_BILLING);

  const prisma = new PrismaClient({
    datasources: {
      db: {
        url: process.env.DATABASE_URL_BILLING
      }
    }
  });

  try {
    console.log('Attempting to connect...');
    await prisma.$connect();
    console.log('Connected successfully!');

    const result = await prisma.$queryRaw`SELECT DATABASE() as current_db`;
    console.log('Current database:', result[0].current_db);
  } catch (error) {
    console.error('Connection error:', error.message);
  } finally {
    await prisma.$disconnect();
  }
}

testConnection();
