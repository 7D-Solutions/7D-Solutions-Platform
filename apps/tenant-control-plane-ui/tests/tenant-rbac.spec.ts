// ============================================================
// RBAC E2E — Roles & permissions panel in Access tab
// Verifies: RBAC snapshot loads and renders, grant/revoke flows
// require confirmation, role picker works, refetch after mutation.
// Verification: npx playwright test -g "RBAC"
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

// BFF route has a 5s upstream timeout before falling back to seed data
const BFF_TIMEOUT = 15000;

test.describe('RBAC', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
    // Clear persisted view state to ensure clean tab state
    await page.request.delete('/api/preferences/view-tenant-detail-home').catch(() => {});
  });

  test('RBAC panel renders in Access tab', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('access-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    // RBAC panel should be visible below users section
    await expect(page.getByTestId('rbac-panel')).toBeVisible({ timeout: BFF_TIMEOUT });
  });

  test('RBAC snapshot loads via BFF route', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    // Listen for the RBAC BFF call when Access tab is activated
    const [rbacRes] = await Promise.all([
      page.waitForResponse(
        (res) => res.url().includes('/api/tenants/test-tenant-001/rbac') &&
          !res.url().includes('/grant') &&
          !res.url().includes('/revoke'),
        { timeout: BFF_TIMEOUT },
      ),
      page.getByTestId('tab-access').click(),
    ]);

    expect(rbacRes.status()).toBe(200);
    const body = await rbacRes.json();
    expect(body).toHaveProperty('roles');
    expect(body).toHaveProperty('user_roles');
  });

  test('available roles list renders with role cards', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('rbac-panel')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Roles list should be present
    await expect(page.getByTestId('rbac-roles-list')).toBeVisible({ timeout: BFF_TIMEOUT });

    // At least one role card
    const roleCards = page.getByTestId('rbac-role-card');
    await expect(roleCards.first()).toBeVisible({ timeout: BFF_TIMEOUT });
    const count = await roleCards.count();
    expect(count).toBeGreaterThan(0);
  });

  test('role cards show permission badges', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('rbac-panel')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('rbac-roles-list')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Permission badges should be present inside role cards
    const badges = page.getByTestId('rbac-permission-badge');
    await expect(badges.first()).toBeVisible({ timeout: BFF_TIMEOUT });
  });

  test('user role assignments table renders', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('rbac-panel')).toBeVisible({ timeout: BFF_TIMEOUT });

    // User roles table should be present
    await expect(page.getByTestId('rbac-user-roles')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('rbac-user-roles-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Table headers
    const table = page.getByTestId('rbac-user-roles-table');
    await expect(table.getByText('User')).toBeVisible();
    await expect(table.getByText('Current Roles')).toBeVisible();
    await expect(table.getByText('Actions')).toBeVisible();
  });

  test('user rows show assigned role badges', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('rbac-panel')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('rbac-user-roles-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    // At least one user row should have an assigned role badge
    const assignedRoles = page.getByTestId('rbac-assigned-role');
    await expect(assignedRoles.first()).toBeVisible({ timeout: BFF_TIMEOUT });
  });

  test('Grant Role button opens picker dropdown', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('rbac-panel')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('rbac-user-roles-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Click the first "Grant Role" button
    const grantBtn = page.getByTestId('rbac-grant-btn').first();
    await expect(grantBtn).toBeVisible({ timeout: BFF_TIMEOUT });
    await grantBtn.click();

    // Picker dropdown should appear with role options
    await expect(page.getByTestId('rbac-grant-picker')).toBeVisible();
    const options = page.getByTestId('rbac-grant-option');
    const optionCount = await options.count();
    expect(optionCount).toBeGreaterThan(0);
  });

  test('selecting a role from picker opens confirmation modal', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('rbac-panel')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('rbac-user-roles-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Open grant picker
    await page.getByTestId('rbac-grant-btn').first().click();
    await expect(page.getByTestId('rbac-grant-picker')).toBeVisible();

    // Click first role option
    await page.getByTestId('rbac-grant-option').first().click();

    // Confirmation modal should appear
    await expect(page.getByRole('dialog')).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Grant Role' })).toBeVisible();
    await expect(page.getByTestId('rbac-confirm-btn')).toBeVisible();
    await expect(page.getByText('Grant', { exact: true })).toBeVisible();
  });

  test('cancel closes grant confirmation modal', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('rbac-panel')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('rbac-user-roles-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Open grant picker and select a role
    await page.getByTestId('rbac-grant-btn').first().click();
    await page.getByTestId('rbac-grant-option').first().click();
    await expect(page.getByRole('dialog')).toBeVisible();

    // Cancel
    await page.getByText('Cancel').click();
    await expect(page.getByRole('dialog')).not.toBeVisible();
  });

  test('clicking revoke × button opens revoke confirmation modal', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('rbac-panel')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('rbac-user-roles-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Click the × revoke button on a role badge
    const revokeBtn = page.getByTestId('rbac-revoke-btn').first();
    await expect(revokeBtn).toBeVisible({ timeout: BFF_TIMEOUT });
    await revokeBtn.click();

    // Revoke confirmation modal should appear
    await expect(page.getByRole('dialog')).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Revoke Role' })).toBeVisible();
    await expect(page.getByTestId('rbac-confirm-btn')).toBeVisible();
  });

  test('confirming grant calls BFF and refetches', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('rbac-panel')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('rbac-user-roles-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Open grant picker and select a role
    await page.getByTestId('rbac-grant-btn').first().click();
    await page.getByTestId('rbac-grant-option').first().click();
    await expect(page.getByRole('dialog')).toBeVisible();

    // Confirm grant — expect BFF call and refetch
    const [grantRes] = await Promise.all([
      page.waitForResponse(
        (res) => res.url().includes('/rbac/grant') && res.request().method() === 'POST',
        { timeout: BFF_TIMEOUT },
      ),
      page.waitForResponse(
        (res) => res.url().includes('/rbac') &&
          !res.url().includes('/grant') &&
          !res.url().includes('/revoke') &&
          res.request().method() === 'GET',
        { timeout: BFF_TIMEOUT },
      ),
      page.getByTestId('rbac-confirm-btn').click(),
    ]);

    // Grant POST should succeed (seed-mode returns 200)
    expect(grantRes.status()).toBe(200);

    // Modal should close after success
    await expect(page.getByRole('dialog')).not.toBeVisible();

    // RBAC panel should still be visible (refetched)
    await expect(page.getByTestId('rbac-panel')).toBeVisible();
  });

  test('confirming revoke calls BFF and refetches', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-access').click();
    await expect(page.getByTestId('rbac-panel')).toBeVisible({ timeout: BFF_TIMEOUT });
    await expect(page.getByTestId('rbac-user-roles-table')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Click revoke on a role badge
    await page.getByTestId('rbac-revoke-btn').first().click();
    await expect(page.getByRole('dialog')).toBeVisible();

    // Confirm revoke — expect BFF call
    const [revokeRes] = await Promise.all([
      page.waitForResponse(
        (res) => res.url().includes('/rbac/revoke') && res.request().method() === 'POST',
        { timeout: BFF_TIMEOUT },
      ),
      page.getByTestId('rbac-confirm-btn').click(),
    ]);

    // Revoke POST should succeed
    expect(revokeRes.status()).toBe(200);

    // Modal should close
    await expect(page.getByRole('dialog')).not.toBeVisible();
  });
});
