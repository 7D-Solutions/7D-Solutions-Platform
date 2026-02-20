// ============================================================
// Feature Override E2E — Grant/revoke override with justification
// Verifies: validation prevents empty justification, override
// modal renders correctly, successful override refetches features.
// Verification: npx playwright test -g "Feature Override"
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

// BFF route has a 5s upstream timeout before falling back to seed data
const BFF_TIMEOUT = 15000;

test.describe('Feature Override', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
    // Clear persisted view state to ensure clean tab state
    await page.request.delete('/api/preferences/view-tenant-detail-home').catch(() => {});
  });

  test('override and revoke buttons appear in features table', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-features').click();
    await expect(page.getByTestId('features-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Plan/bundle rows should have "Override" button
    const grantBtns = page.getByTestId('override-grant-btn');
    await expect(grantBtns.first()).toBeVisible({ timeout: BFF_TIMEOUT });

    // Override rows should have "Revoke" button (seed data has override rows)
    const revokeBtns = page.getByTestId('override-revoke-btn');
    await expect(revokeBtns.first()).toBeVisible({ timeout: BFF_TIMEOUT });
  });

  test('Actions column header is visible', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-features').click();
    await expect(page.getByTestId('features-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    await expect(page.getByTestId('features-table').getByText('Actions')).toBeVisible();
  });

  test('clicking Override opens the grant modal', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-features').click();
    await expect(page.getByTestId('features-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Click first "Override" button (on a plan/bundle row)
    await page.getByTestId('override-grant-btn').first().click();

    // Modal should be visible with Grant Override title
    await expect(page.getByRole('dialog')).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Grant Override' })).toBeVisible();
    await expect(page.getByTestId('override-justification')).toBeVisible();
    await expect(page.getByTestId('override-confirm-btn')).toBeVisible();
  });

  test('clicking Revoke opens the revoke modal', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-features').click();
    await expect(page.getByTestId('features-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Click first "Revoke" button (on an override row)
    await page.getByTestId('override-revoke-btn').first().click();

    // Modal should be visible with Revoke Override title
    await expect(page.getByRole('dialog')).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Revoke Override' })).toBeVisible();
    await expect(page.getByTestId('override-justification')).toBeVisible();
    await expect(page.getByTestId('override-confirm-btn')).toBeVisible();
  });

  test('submitting without justification shows validation error', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-features').click();
    await expect(page.getByTestId('features-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Open override modal
    await page.getByTestId('override-grant-btn').first().click();
    await expect(page.getByRole('dialog')).toBeVisible();

    // Click confirm without entering justification
    await page.getByTestId('override-confirm-btn').click();

    // Validation error should appear
    const errorEl = page.getByTestId('override-validation-error');
    await expect(errorEl).toBeVisible();
    await expect(errorEl).not.toHaveText('');
  });

  test('cancel closes the override modal', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-features').click();
    await expect(page.getByTestId('features-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Open override modal
    await page.getByTestId('override-grant-btn').first().click();
    await expect(page.getByRole('dialog')).toBeVisible();

    // Click Cancel
    await page.getByText('Cancel').click();

    // Modal should close
    await expect(page.getByRole('dialog')).not.toBeVisible();
  });

  test('successful grant override refetches effective features', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-features').click();
    await expect(page.getByTestId('features-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Open override modal on a plan/bundle row
    await page.getByTestId('override-grant-btn').first().click();
    await expect(page.getByRole('dialog')).toBeVisible();

    // Fill in justification
    await page.getByTestId('override-justification').fill('Approved for enterprise pilot program');

    // Listen for the BFF override POST and subsequent refetch GET
    const [overrideRes] = await Promise.all([
      page.waitForResponse(
        (res) => res.url().includes('/features/override') && res.request().method() === 'POST',
        { timeout: BFF_TIMEOUT },
      ),
      page.waitForResponse(
        (res) => res.url().includes('/features/effective') && res.request().method() === 'GET',
        { timeout: BFF_TIMEOUT },
      ),
      page.getByTestId('override-confirm-btn').click(),
    ]);

    // Override POST should succeed (seed-mode returns 200)
    expect(overrideRes.status()).toBe(200);

    // Modal should close after success
    await expect(page.getByRole('dialog')).not.toBeVisible();

    // Features table should still be visible (refetched)
    await expect(page.getByTestId('features-table')).toBeVisible();
  });

  test('successful revoke override refetches effective features', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-features').click();
    await expect(page.getByTestId('features-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Open revoke modal on an override row
    await page.getByTestId('override-revoke-btn').first().click();
    await expect(page.getByRole('dialog')).toBeVisible();

    // Fill in justification
    await page.getByTestId('override-justification').fill('Override no longer needed after plan upgrade');

    // Listen for the BFF override POST
    const [overrideRes] = await Promise.all([
      page.waitForResponse(
        (res) => res.url().includes('/features/override') && res.request().method() === 'POST',
        { timeout: BFF_TIMEOUT },
      ),
      page.getByTestId('override-confirm-btn').click(),
    ]);

    // Override POST should succeed
    expect(overrideRes.status()).toBe(200);

    // Modal should close
    await expect(page.getByRole('dialog')).not.toBeVisible();
  });

  test('justification character counter updates as user types', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-features').click();
    await expect(page.getByTestId('features-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    await page.getByTestId('override-grant-btn').first().click();
    await expect(page.getByRole('dialog')).toBeVisible();

    // Initially 0/500
    await expect(page.getByText('0/500')).toBeVisible();

    // Type some text
    await page.getByTestId('override-justification').fill('Test reason');
    await expect(page.getByText('11/500')).toBeVisible();
  });

  test('validation error clears when user starts typing', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-features').click();
    await expect(page.getByTestId('features-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    await page.getByTestId('override-grant-btn').first().click();
    await expect(page.getByRole('dialog')).toBeVisible();

    // Submit empty to trigger validation
    await page.getByTestId('override-confirm-btn').click();
    const errorEl = page.getByTestId('override-validation-error');
    await expect(errorEl).not.toHaveText('');

    // Start typing — error should clear
    await page.getByTestId('override-justification').fill('A');
    await expect(errorEl).toHaveText('');
  });
});
