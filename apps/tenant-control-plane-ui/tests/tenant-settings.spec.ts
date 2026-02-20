// ============================================================
// Tenant Settings E2E — lifecycle actions, re-auth gating
// Verifies: Settings tab renders, suspend/activate confirmation,
// terminate requires reason + re-auth, BFF routes called.
// Verification: npx playwright test -g "Tenant Settings|Terminate|Re-auth"
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

const BFF_TIMEOUT = 15000;

test.describe('Tenant Settings', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
    // Clear persisted view state
    await page.request.delete('/api/preferences/view-tenant-detail-home').catch(() => {});
  });

  test('Settings tab renders lifecycle section', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Lifecycle section is present
    await expect(page.getByTestId('lifecycle-section')).toBeVisible();
    await expect(page.getByText('Lifecycle Actions')).toBeVisible();
    await expect(page.getByText('Current status:')).toBeVisible();
  });

  test('Settings tab renders plan change and config sections', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    await expect(page.getByTestId('plan-change-section')).toBeVisible();
    await expect(page.getByTestId('change-plan-btn')).toBeDisabled();
    await expect(page.getByTestId('account-config-section')).toBeVisible();
  });

  test('lifecycle buttons shown based on tenant status', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    // test-tenant-001 is typically active — expect suspend and terminate buttons
    const suspendBtn = page.getByTestId('suspend-btn');
    const activateBtn = page.getByTestId('activate-btn');
    const terminateBtn = page.getByTestId('terminate-btn');

    // At least one lifecycle button should be visible
    await expect(
      suspendBtn.or(activateBtn).or(terminateBtn).first()
    ).toBeVisible({ timeout: 5000 });
  });

  test('suspend action opens confirmation and calls BFF', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    const suspendBtn = page.getByTestId('suspend-btn');
    // Skip if suspend isn't available (tenant may not be active)
    if (!(await suspendBtn.isVisible().catch(() => false))) {
      test.skip();
      return;
    }

    await suspendBtn.click();
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });
    await expect(page.getByRole('heading', { name: 'Suspend Tenant' })).toBeVisible();
    await expect(page.getByText(/Are you sure you want to suspend/)).toBeVisible();

    // Confirm the suspend
    const [suspendRes] = await Promise.all([
      page.waitForResponse(
        (res) => res.url().includes('/suspend') && res.request().method() === 'POST',
        { timeout: BFF_TIMEOUT },
      ),
      page.getByTestId('confirm-lifecycle-btn').click(),
    ]);

    expect(suspendRes.status()).toBe(200);
    await expect(page.getByRole('dialog')).not.toBeVisible({ timeout: 5000 });
  });

  test('activate action opens confirmation and calls BFF', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Activate is only available for suspended tenants — skip in seed mode
    const activateBtn = page.getByTestId('activate-btn');
    if (!(await activateBtn.isVisible().catch(() => false))) {
      test.skip();
      return;
    }

    await activateBtn.click();
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });
    await expect(page.getByRole('heading', { name: 'Activate Tenant' })).toBeVisible();

    const [activateRes] = await Promise.all([
      page.waitForResponse(
        (res) => res.url().includes('/activate') && res.request().method() === 'POST',
        { timeout: BFF_TIMEOUT },
      ),
      page.getByTestId('confirm-lifecycle-btn').click(),
    ]);

    expect(activateRes.status()).toBe(200);
    await expect(page.getByRole('dialog')).not.toBeVisible({ timeout: 5000 });
  });

  test('cancel closes lifecycle confirmation modal', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Click whichever lifecycle button is available
    const suspendBtn = page.getByTestId('suspend-btn');
    const activateBtn = page.getByTestId('activate-btn');
    const btn = (await suspendBtn.isVisible().catch(() => false)) ? suspendBtn : activateBtn;
    if (!(await btn.isVisible().catch(() => false))) {
      test.skip();
      return;
    }

    await btn.click();
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });

    await page.getByRole('button', { name: 'Cancel' }).click();
    await expect(page.getByRole('dialog')).not.toBeVisible({ timeout: 5000 });
  });
});

test.describe('Terminate with Re-auth', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
    await page.request.delete('/api/preferences/view-tenant-detail-home').catch(() => {});
  });

  test('terminate opens reason step and requires non-empty reason', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    const terminateBtn = page.getByTestId('terminate-btn');
    if (!(await terminateBtn.isVisible().catch(() => false))) {
      test.skip();
      return;
    }

    await terminateBtn.click();
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });
    await expect(page.getByRole('heading', { name: 'Terminate Tenant' })).toBeVisible();
    await expect(page.getByText('Reason for termination')).toBeVisible();

    // Next button should be disabled with empty reason
    await expect(page.getByTestId('terminate-next-btn')).toBeDisabled();

    // Type a reason
    await page.getByTestId('terminate-reason-input').fill('Test termination reason');
    await expect(page.getByTestId('terminate-next-btn')).toBeEnabled();
  });

  test('terminate step 2: re-auth requires password', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    const terminateBtn = page.getByTestId('terminate-btn');
    if (!(await terminateBtn.isVisible().catch(() => false))) {
      test.skip();
      return;
    }

    // Step 1: reason
    await terminateBtn.click();
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });
    await page.getByTestId('terminate-reason-input').fill('Compliance requirement');
    await page.getByTestId('terminate-next-btn').click();

    // Step 2: re-auth
    await expect(page.getByText('enter your password')).toBeVisible({ timeout: 5000 });
    await expect(page.getByTestId('reauth-verify-btn')).toBeDisabled();

    // Enter password
    await page.getByTestId('reauth-password-input').fill('admin-password');
    await expect(page.getByTestId('reauth-verify-btn')).toBeEnabled();
  });

  test('re-auth verify calls BFF and advances to final confirm', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    const terminateBtn = page.getByTestId('terminate-btn');
    if (!(await terminateBtn.isVisible().catch(() => false))) {
      test.skip();
      return;
    }

    // Step 1: reason
    await terminateBtn.click();
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });
    await page.getByTestId('terminate-reason-input').fill('End of contract');
    await page.getByTestId('terminate-next-btn').click();

    // Step 2: re-auth
    await expect(page.getByTestId('reauth-password-input')).toBeVisible({ timeout: 5000 });
    await page.getByTestId('reauth-password-input').fill('admin-password');

    const [reauthRes] = await Promise.all([
      page.waitForResponse(
        (res) => res.url().includes('/api/auth/reauth') && res.request().method() === 'POST',
        { timeout: BFF_TIMEOUT },
      ),
      page.getByTestId('reauth-verify-btn').click(),
    ]);

    expect(reauthRes.status()).toBe(200);

    // Step 3: final confirm
    await expect(page.getByTestId('terminate-warning')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('cannot be undone')).toBeVisible();
    await expect(page.getByTestId('confirm-terminate-btn')).toBeVisible();
  });

  test('final terminate calls BFF and closes modal', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    const terminateBtn = page.getByTestId('terminate-btn');
    if (!(await terminateBtn.isVisible().catch(() => false))) {
      test.skip();
      return;
    }

    // Step 1: reason
    await terminateBtn.click();
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });
    await page.getByTestId('terminate-reason-input').fill('Business closure');
    await page.getByTestId('terminate-next-btn').click();

    // Step 2: re-auth
    await expect(page.getByTestId('reauth-password-input')).toBeVisible({ timeout: 5000 });
    await page.getByTestId('reauth-password-input').fill('admin-password');
    await Promise.all([
      page.waitForResponse(
        (res) => res.url().includes('/api/auth/reauth'),
        { timeout: BFF_TIMEOUT },
      ),
      page.getByTestId('reauth-verify-btn').click(),
    ]);

    // Step 3: confirm terminate
    await expect(page.getByTestId('confirm-terminate-btn')).toBeVisible({ timeout: 5000 });

    const [terminateRes] = await Promise.all([
      page.waitForResponse(
        (res) => res.url().includes('/terminate') && res.request().method() === 'POST'
          && !res.url().includes('/reauth'),
        { timeout: BFF_TIMEOUT },
      ),
      page.getByTestId('confirm-terminate-btn').click(),
    ]);

    expect(terminateRes.status()).toBe(200);
    await expect(page.getByRole('dialog')).not.toBeVisible({ timeout: 5000 });
  });

  test('cancel at reason step closes terminate modal', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    const terminateBtn = page.getByTestId('terminate-btn');
    if (!(await terminateBtn.isVisible().catch(() => false))) {
      test.skip();
      return;
    }

    await terminateBtn.click();
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });

    await page.getByRole('button', { name: 'Cancel' }).click();
    await expect(page.getByRole('dialog')).not.toBeVisible({ timeout: 5000 });
  });

  test('back button at re-auth step returns to reason step', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    const terminateBtn = page.getByTestId('terminate-btn');
    if (!(await terminateBtn.isVisible().catch(() => false))) {
      test.skip();
      return;
    }

    // Go to reason → re-auth
    await terminateBtn.click();
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });
    await page.getByTestId('terminate-reason-input').fill('Test reason');
    await page.getByTestId('terminate-next-btn').click();
    await expect(page.getByTestId('reauth-password-input')).toBeVisible({ timeout: 5000 });

    // Click back
    await page.getByRole('button', { name: 'Back' }).click();

    // Should be back at reason step with reason preserved
    await expect(page.getByTestId('terminate-reason-input')).toBeVisible({ timeout: 5000 });
    await expect(page.getByTestId('terminate-reason-input')).toHaveValue('Test reason');
  });
});
