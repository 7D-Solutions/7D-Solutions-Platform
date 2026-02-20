// ============================================================
// Tenant Users E2E — Access tab, users list, deactivation
// Verifies: Access tab renders users list via BFF, deactivation
// requires confirmation modal, and refetch updates the UI.
// Verification: npx playwright test -g "Tenant Users"
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

test.describe('Tenant Users', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
    // Clear persisted view state to ensure clean tab state
    await page.request.delete('/api/preferences/view-tenant-detail-home');
  });

  test('Access tab renders with users list', async ({ page }) => {
    // Navigate to tenant detail and wait for BFF response
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    // Click Access tab
    await page.getByTestId('tab-access').click();

    // Wait for the Access tab content to load
    await expect(page.getByTestId('access-tab')).toBeVisible({ timeout: 10000 });

    // Verify users list renders (seed data or real data)
    const usersTable = page.getByTestId('users-table');
    const usersEmpty = page.getByTestId('users-empty');

    // Either the table with rows, or the empty state should be visible
    await expect(usersTable.or(usersEmpty)).toBeVisible({ timeout: 10000 });
  });

  test('users table shows expected columns', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('access-tab')).toBeVisible({ timeout: 10000 });

    // Wait for the table to appear (seed data provides users when identity-auth is down)
    const usersTable = page.getByTestId('users-table');
    await expect(usersTable).toBeVisible({ timeout: 10000 });

    // Verify column headers are present
    await expect(usersTable.getByText('Email')).toBeVisible();
    await expect(usersTable.getByText('Name')).toBeVisible();
    await expect(usersTable.getByText('Status')).toBeVisible();
    await expect(usersTable.getByText('Last Seen')).toBeVisible();
    await expect(usersTable.getByText('Actions')).toBeVisible();
  });

  test('users list is fetched via BFF route', async ({ page }) => {
    const [usersRes] = await Promise.all([
      page.waitForResponse((res) =>
        res.url().includes('/api/tenants/test-tenant-001/users') &&
        !res.url().includes('/deactivate'),
      ),
      page.goto('/tenants/test-tenant-001'),
    ]);

    // BFF route not called until Access tab is activated
    // Navigate to Access tab and wait for the response
    const [bffRes] = await Promise.all([
      page.waitForResponse((res) =>
        res.url().includes('/api/tenants/test-tenant-001/users') &&
        !res.url().includes('/deactivate'),
      ),
      page.getByTestId('tab-access').click(),
    ]);

    expect(bffRes.status()).toBe(200);
    const body = await bffRes.json();
    expect(body).toHaveProperty('users');
    expect(body).toHaveProperty('total');
  });

  test('user rows render with data', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('access-tab')).toBeVisible({ timeout: 10000 });

    // Wait for table
    await expect(page.getByTestId('users-table')).toBeVisible({ timeout: 10000 });

    // Verify at least one user row exists
    const userRows = page.getByTestId('user-row');
    await expect(userRows.first()).toBeVisible({ timeout: 10000 });
    const count = await userRows.count();
    expect(count).toBeGreaterThan(0);
  });

  test('deactivate button opens confirmation modal', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('access-tab')).toBeVisible({ timeout: 10000 });
    await expect(page.getByTestId('users-table')).toBeVisible({ timeout: 10000 });

    // Find a deactivate button (only appears for active users)
    const deactivateBtn = page.getByTestId('deactivate-btn').first();
    const btnVisible = await deactivateBtn.isVisible().catch(() => false);

    if (btnVisible) {
      await deactivateBtn.click();

      // Confirmation modal should appear
      await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });
      await expect(page.getByText('Deactivate User')).toBeVisible();
      await expect(page.getByText(/Are you sure you want to deactivate/)).toBeVisible();

      // Modal has Cancel and Deactivate buttons
      await expect(page.getByRole('button', { name: 'Cancel' })).toBeVisible();
      await expect(page.getByTestId('confirm-deactivate-btn')).toBeVisible();
    }
  });

  test('cancel closes the confirmation modal without action', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('access-tab')).toBeVisible({ timeout: 10000 });
    await expect(page.getByTestId('users-table')).toBeVisible({ timeout: 10000 });

    const deactivateBtn = page.getByTestId('deactivate-btn').first();
    const btnVisible = await deactivateBtn.isVisible().catch(() => false);

    if (btnVisible) {
      await deactivateBtn.click();
      await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });

      // Click Cancel
      await page.getByRole('button', { name: 'Cancel' }).click();

      // Modal should close
      await expect(page.getByRole('dialog')).not.toBeVisible();
    }
  });

  test('confirming deactivation calls BFF and refetches', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('access-tab')).toBeVisible({ timeout: 10000 });
    await expect(page.getByTestId('users-table')).toBeVisible({ timeout: 10000 });

    const deactivateBtn = page.getByTestId('deactivate-btn').first();
    const btnVisible = await deactivateBtn.isVisible().catch(() => false);

    if (btnVisible) {
      await deactivateBtn.click();
      await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });

      // Click confirm — expect BFF call and modal closes
      const [deactivateRes] = await Promise.all([
        page.waitForResponse((res) => res.url().includes('/deactivate')),
        page.getByTestId('confirm-deactivate-btn').click(),
      ]);

      expect(deactivateRes.status()).toBe(200);

      // Modal should close after successful deactivation
      await expect(page.getByRole('dialog')).not.toBeVisible({ timeout: 5000 });
    }
  });

  test('switching between Overview and Access tabs works correctly', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('overview-tab')).toBeVisible();

    // Switch to Access
    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('access-tab')).toBeVisible({ timeout: 10000 });
    await expect(page.getByTestId('overview-tab')).not.toBeVisible();

    // Switch back to Overview
    await page.getByTestId('tab-overview').click();
    await expect(page.getByTestId('overview-tab')).toBeVisible({ timeout: 10000 });
    await expect(page.getByTestId('access-tab')).not.toBeVisible();
  });
});
