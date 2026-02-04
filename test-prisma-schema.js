// Quick test to verify Prisma schema has app_id field
const { PrismaClient } = require('./node_modules/.prisma/ar');

const client = new PrismaClient();

// Try to get the model fields
const refundsModel = client._runtimeDataModel.models.billing_refunds;

console.log('billing_refunds fields:', Object.keys(refundsModel.fields));
console.log('\napp_id field definition:', refundsModel.fields.app_id);

client.$disconnect();
