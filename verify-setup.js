#!/usr/bin/env node

/**
 * AR Setup Verification Script
 * Run this to verify your accounts receivable module is configured correctly
 */

const path = require('path');

console.log('🔍 Verifying @fireproof/ar setup...\n');

// Check 1: Environment variables
console.log('1. Checking environment variables...');
const requiredEnvVars = [
  'DATABASE_URL_BILLING',
  'TILLED_SECRET_KEY_TRASHTECH',
  'TILLED_ACCOUNT_ID_TRASHTECH',
  'TILLED_WEBHOOK_SECRET_TRASHTECH',
  'TILLED_SANDBOX'
];

let envCheckPassed = true;
requiredEnvVars.forEach(varName => {
  if (!process.env[varName]) {
    console.log(`   ❌ Missing: ${varName}`);
    envCheckPassed = false;
  } else {
    console.log(`   ✅ ${varName} is set`);
  }
});

if (!envCheckPassed) {
  console.log('\n⚠️  Set missing variables in your .env file');
  console.log('   See .env.example for template\n');
  process.exit(1);
}

// Check 2: Prisma client generated
console.log('\n2. Checking Prisma client...');
try {
  require('@prisma/client');
  console.log('   ✅ Prisma client found');
} catch (err) {
  console.log('   ❌ Prisma client not generated');
  console.log('   Run: npm run prisma:generate\n');
  process.exit(1);
}

// Check 3: Can import billing service
console.log('\n3. Checking billing service...');
try {
  const { BillingService } = require('./backend/src');
  console.log('   ✅ BillingService imports successfully');

  const service = new BillingService();
  console.log('   ✅ BillingService instantiates successfully');
} catch (err) {
  console.log(`   ❌ Failed to import BillingService: ${err.message}\n`);
  process.exit(1);
}

// Check 4: Can connect to billing database
console.log('\n4. Testing database connection...');
(async () => {
  try {
    const { billingPrisma } = require('./backend/src/prisma');

    // Try to connect
    await billingPrisma.$connect();
    console.log('   ✅ Database connection successful');

    // Check if tables exist
    const tables = await billingPrisma.$queryRaw`
      SELECT table_name
      FROM information_schema.tables
      WHERE table_schema = DATABASE()
      AND table_name LIKE 'billing_%'
    `;

    if (tables.length === 3) {
      console.log('   ✅ All 3 billing tables found:');
      tables.forEach(t => console.log(`      - ${t.TABLE_NAME || t.table_name}`));
    } else {
      console.log(`   ⚠️  Found ${tables.length} tables (expected 3)`);
      console.log('   Run: npm run prisma:migrate');
    }

    await billingPrisma.$disconnect();

    console.log('\n✅ All checks passed! Billing module is ready.\n');
    console.log('Next steps:');
    console.log('  1. Mount routes in your app (see APP-INTEGRATION-EXAMPLE.md)');
    console.log('  2. Run sandbox tests (see SANDBOX-TEST-CHECKLIST.md)');
    console.log('  3. Deploy to production (see PRODUCTION-OPS.md)\n');

  } catch (err) {
    console.log(`   ❌ Database connection failed: ${err.message}`);
    console.log('\n   Possible issues:');
    console.log('   - Database does not exist (run: CREATE DATABASE billing_db;)');
    console.log('   - Wrong DATABASE_URL_BILLING');
    console.log('   - MySQL not running');
    console.log('   - Migrations not run (run: npm run prisma:migrate)\n');
    process.exit(1);
  }
})();
