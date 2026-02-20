// ============================================================
// Admin Tools — Playwright E2E
// Validates: page renders, form validation, confirmation modal,
// and either success result or deterministic not-available state.
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

test.describe('Admin Tools', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
  });

  test('admin tools page renders with both tool cards', async ({ page }) => {
    await page.goto('/system/admin-tools');
    await expect(page.getByTestId('admin-tools-page')).toBeVisible({ timeout: 15000 });
    await expect(page.getByRole('heading', { name: /admin tools/i })).toBeVisible();
    await expect(page.getByTestId('run-billing-tool')).toBeVisible();
    await expect(page.getByTestId('reconcile-mapping-tool')).toBeVisible();
  });

  test('sidebar navigation includes Admin Tools link', async ({ page }) => {
    await page.goto('/tenants');
    await expect(page.getByText('Admin Tools')).toBeVisible({ timeout: 15000 });
  });

  test('can navigate to admin tools from sidebar', async ({ page }) => {
    await page.goto('/tenants');
    await expect(page.getByText('Admin Tools')).toBeVisible({ timeout: 15000 });
    await page.getByText('Admin Tools').click();
    await expect(page).toHaveURL(/\/system\/admin-tools/);
    await expect(page.getByTestId('admin-tools-page')).toBeVisible({ timeout: 15000 });
  });

  // ── Run Billing Tool ────────────────────────────────────────

  test('run billing: shows validation error when reason is empty', async ({ page }) => {
    await page.goto('/system/admin-tools');
    await expect(page.getByTestId('admin-tools-page')).toBeVisible({ timeout: 15000 });

    await page.getByTestId('billing-submit-btn').click();

    // Reason is required — validation error should appear
    await expect(page.getByText('Reason is required')).toBeVisible({ timeout: 5000 });
  });

  test('run billing: opens confirmation modal with summary', async ({ page }) => {
    await page.goto('/system/admin-tools');
    await expect(page.getByTestId('admin-tools-page')).toBeVisible({ timeout: 15000 });

    // Fill in the form
    await page.getByTestId('billing-reason').fill('Monthly billing cycle test');

    // Submit to open confirmation
    await page.getByTestId('billing-submit-btn').click();

    // Confirmation modal should appear
    await expect(page.getByTestId('billing-confirm-summary')).toBeVisible({ timeout: 5000 });
    await expect(page.getByTestId('confirm-tenant-value')).toHaveText('All tenants');
    await expect(page.getByTestId('confirm-reason-value')).toHaveText('Monthly billing cycle test');
  });

  test('run billing: can cancel confirmation modal', async ({ page }) => {
    await page.goto('/system/admin-tools');
    await expect(page.getByTestId('admin-tools-page')).toBeVisible({ timeout: 15000 });

    await page.getByTestId('billing-reason').fill('Test reason');
    await page.getByTestId('billing-submit-btn').click();
    await expect(page.getByTestId('billing-confirm-summary')).toBeVisible({ timeout: 5000 });

    // Cancel
    await page.getByRole('button', { name: 'Cancel' }).click();
    await expect(page.getByTestId('billing-confirm-summary')).not.toBeVisible();
  });

  test('run billing: confirm executes and shows result', async ({ page }) => {
    await page.goto('/system/admin-tools');
    await expect(page.getByTestId('admin-tools-page')).toBeVisible({ timeout: 15000 });

    await page.getByTestId('billing-reason').fill('End-of-month billing run');
    await page.getByTestId('billing-submit-btn').click();
    await expect(page.getByTestId('billing-confirm-summary')).toBeVisible({ timeout: 5000 });

    // Confirm
    await page.getByTestId('billing-confirm-btn').click();

    // Should show either success or not-available result
    const successOrNotAvailable = page
      .getByTestId('tool-result-success')
      .or(page.getByTestId('tool-result-not-available'));
    await expect(successOrNotAvailable).toBeVisible({ timeout: 15000 });
  });

  test('run billing: shows tenant ID in confirmation when provided', async ({ page }) => {
    await page.goto('/system/admin-tools');
    await expect(page.getByTestId('admin-tools-page')).toBeVisible({ timeout: 15000 });

    await page.getByTestId('billing-tenant-id').fill('tenant-abc-123');
    await page.getByTestId('billing-reason').fill('Specific tenant billing');
    await page.getByTestId('billing-submit-btn').click();

    await expect(page.getByTestId('confirm-tenant-value')).toHaveText('tenant-abc-123');
  });

  // ── Reconcile Tenant Mapping Tool ────────────────────────────

  test('reconcile mapping: shows validation error when fields are empty', async ({ page }) => {
    await page.goto('/system/admin-tools');
    await expect(page.getByTestId('admin-tools-page')).toBeVisible({ timeout: 15000 });

    await page.getByTestId('reconcile-submit-btn').click();

    // Both tenant ID and reason are required
    await expect(page.getByText('Tenant ID is required')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Reason is required')).toBeVisible({ timeout: 5000 });
  });

  test('reconcile mapping: opens confirmation modal with summary', async ({ page }) => {
    await page.goto('/system/admin-tools');
    await expect(page.getByTestId('admin-tools-page')).toBeVisible({ timeout: 15000 });

    await page.getByTestId('reconcile-tenant-id').fill('tenant-xyz-789');
    await page.getByTestId('reconcile-reason').fill('Mapping drift detected');
    await page.getByTestId('reconcile-submit-btn').click();

    await expect(page.getByTestId('reconcile-confirm-summary')).toBeVisible({ timeout: 5000 });
    await expect(page.getByTestId('confirm-reconcile-tenant')).toHaveText('tenant-xyz-789');
    await expect(page.getByTestId('confirm-reconcile-reason')).toHaveText('Mapping drift detected');
  });

  test('reconcile mapping: confirm executes and shows result', async ({ page }) => {
    await page.goto('/system/admin-tools');
    await expect(page.getByTestId('admin-tools-page')).toBeVisible({ timeout: 15000 });

    await page.getByTestId('reconcile-tenant-id').fill('tenant-xyz-789');
    await page.getByTestId('reconcile-reason').fill('Re-sync after migration');
    await page.getByTestId('reconcile-submit-btn').click();

    await expect(page.getByTestId('reconcile-confirm-summary')).toBeVisible({ timeout: 5000 });
    await page.getByTestId('reconcile-confirm-btn').click();

    // Should show either success or not-available result
    const successOrNotAvailable = page
      .getByTestId('tool-result-success')
      .or(page.getByTestId('tool-result-not-available'));
    await expect(successOrNotAvailable).toBeVisible({ timeout: 15000 });
  });
});
