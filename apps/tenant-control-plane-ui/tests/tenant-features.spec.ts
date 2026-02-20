// ============================================================
// Tenant Features E2E — Features tab, effective entitlements
// Verifies: Features tab renders entitlements with source
// attribution via BFF aggregation endpoint.
// Verification: npx playwright test -g "Tenant Features"
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

// BFF route has a 5s upstream timeout before falling back to seed data
const BFF_TIMEOUT = 15000;

test.describe('Tenant Features', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
    // Clear persisted view state to ensure clean tab state
    await page.request.delete('/api/preferences/view-tenant-detail-home').catch(() => {});
  });

  test('Features tab renders with entitlements table', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    // Click Features tab
    await page.getByTestId('tab-features').click();
    await expect(page.getByTestId('features-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Either the table or the empty state should be visible
    const featuresTable = page.getByTestId('features-table');
    const featuresEmpty = page.getByTestId('features-empty');
    await expect(featuresTable.or(featuresEmpty)).toBeVisible({ timeout: BFF_TIMEOUT });
  });

  test('features table shows expected columns', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-features').click();
    await expect(page.getByTestId('features-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    const featuresTable = page.getByTestId('features-table');
    await expect(featuresTable).toBeVisible({ timeout: BFF_TIMEOUT });

    // Verify column headers
    await expect(featuresTable.getByText('Code')).toBeVisible();
    await expect(featuresTable.getByText('Name')).toBeVisible();
    await expect(featuresTable.getByText('Granted')).toBeVisible();
    await expect(featuresTable.getByText('Source')).toBeVisible();
    await expect(featuresTable.getByText('Details')).toBeVisible();
  });

  test('features are fetched via BFF aggregation endpoint', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    const [bffRes] = await Promise.all([
      page.waitForResponse(
        (res) => res.url().includes('/api/tenants/test-tenant-001/features/effective'),
        { timeout: BFF_TIMEOUT },
      ),
      page.getByTestId('tab-features').click(),
    ]);

    expect(bffRes.status()).toBe(200);
    const body = await bffRes.json();
    expect(body).toHaveProperty('entitlements');
    expect(body).toHaveProperty('total');
  });

  test('feature rows render with source badges', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-features').click();
    await expect(page.getByTestId('features-tab')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('features-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Verify at least one feature row exists
    const featureRows = page.getByTestId('feature-row');
    await expect(featureRows.first()).toBeVisible({ timeout: BFF_TIMEOUT });
    const count = await featureRows.count();
    expect(count).toBeGreaterThan(0);

    // Verify source badges are rendered (Plan, Bundle, or Override)
    const sourceBadges = page.getByTestId('source-badge');
    await expect(sourceBadges.first()).toBeVisible();
  });

  test('source badges include Plan, Bundle, and Override labels', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-features').click();
    await expect(page.getByTestId('features-tab')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('features-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Seed data includes all three source types
    await expect(page.getByTestId('source-badge').filter({ hasText: 'Plan' }).first()).toBeVisible();
    await expect(page.getByTestId('source-badge').filter({ hasText: 'Bundle' }).first()).toBeVisible();
    await expect(page.getByTestId('source-badge').filter({ hasText: 'Override' }).first()).toBeVisible();
  });

  test('override rows show justification', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-features').click();
    await expect(page.getByTestId('features-tab')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('features-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Seed data includes overrides with justification text
    const justification = page.getByTestId('feature-justification').first();
    await expect(justification).toBeVisible({ timeout: BFF_TIMEOUT });
  });

  test('search filter narrows entitlements', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-features').click();
    await expect(page.getByTestId('features-tab')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('features-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    const allRows = page.getByTestId('feature-row');
    const initialCount = await allRows.count();
    expect(initialCount).toBeGreaterThan(1);

    // Type a search term
    await page.getByTestId('features-search').fill('sso');

    // Wait for debounce and re-render — filtered count should be less
    await page.waitForTimeout(500);
    const filteredCount = await allRows.count();
    expect(filteredCount).toBeLessThan(initialCount);
    expect(filteredCount).toBeGreaterThan(0);
  });

  test('source filter narrows to selected source', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-features').click();
    await expect(page.getByTestId('features-tab')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('features-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Filter to override only
    await page.getByTestId('features-source-filter').selectOption('override');

    // All visible source badges should say "Override"
    const sourceBadges = page.getByTestId('source-badge');
    const count = await sourceBadges.count();
    expect(count).toBeGreaterThan(0);
    for (let i = 0; i < count; i++) {
      await expect(sourceBadges.nth(i)).toHaveText('Override');
    }
  });

  test('switching between Overview and Features tabs works correctly', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('overview-tab')).toBeVisible();

    // Switch to Features
    await page.getByTestId('tab-features').click();
    await expect(page.getByTestId('features-tab')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('overview-tab')).not.toBeVisible();

    // Switch back to Overview
    await page.getByTestId('tab-overview').click();
    await expect(page.getByTestId('overview-tab')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('features-tab')).not.toBeVisible();
  });

  test('features filters are visible', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-features').click();
    await expect(page.getByTestId('features-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Verify filter controls are present
    await expect(page.getByTestId('features-filters')).toBeVisible();
    await expect(page.getByTestId('features-search')).toBeVisible();
    await expect(page.getByTestId('features-source-filter')).toBeVisible();
  });
});
