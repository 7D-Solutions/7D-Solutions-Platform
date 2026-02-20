// ============================================================
// Plan Assignment E2E — change tenant plan with effective date
// Verifies: modal opens, plan select + effective date validation,
// form submission calls BFF, plan summary refetches on success.
// Verification: npx playwright test -g "Plan Assignment"
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

const BFF_TIMEOUT = 15000;

test.describe('Plan Assignment', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
    // Clear persisted view state
    await page.request.delete('/api/preferences/view-tenant-detail-home').catch(() => {});
  });

  test('Change Plan button opens modal with plan select and effective date', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Click Change Plan
    await page.getByTestId('change-plan-btn').click();
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });
    await expect(page.getByRole('heading', { name: 'Change Plan' })).toBeVisible();

    // Form controls are present
    await expect(page.getByTestId('plan-select')).toBeVisible();
    await expect(page.getByTestId('effective-date-input')).toBeVisible();
    await expect(page.getByTestId('confirm-plan-change-btn')).toBeVisible();
  });

  test('cancel closes the plan change modal', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    await page.getByTestId('change-plan-btn').click();
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });

    await page.getByRole('button', { name: 'Cancel' }).click();
    await expect(page.getByRole('dialog')).not.toBeVisible({ timeout: 5000 });
  });

  test('form shows validation error when plan not selected', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    await page.getByTestId('change-plan-btn').click();
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });

    // Effective date has a default (today) so only plan needs selecting
    // Submit without selecting a plan
    await page.getByTestId('confirm-plan-change-btn').click();

    // Validation error should appear
    await expect(page.getByText('Plan is required')).toBeVisible({ timeout: 3000 });
  });

  test('effective date defaults to today', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    await page.getByTestId('change-plan-btn').click();
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });

    const dateInput = page.getByTestId('effective-date-input');
    const value = await dateInput.inputValue();
    // Should be today in YYYY-MM-DD format
    const today = new Date();
    const yyyy = today.getFullYear();
    const mm = String(today.getMonth() + 1).padStart(2, '0');
    const dd = String(today.getDate()).padStart(2, '0');
    expect(value).toBe(`${yyyy}-${mm}-${dd}`);
  });

  test('plan select is populated with active plans', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    await page.getByTestId('change-plan-btn').click();
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });

    // Wait for plans to load — select should have options
    const planSelect = page.getByTestId('plan-select');
    await expect(planSelect).toBeVisible({ timeout: BFF_TIMEOUT });

    // The select should have at least one option beyond the placeholder
    const options = planSelect.locator('option:not([disabled])');
    const count = await options.count();
    expect(count).toBeGreaterThanOrEqual(1);
  });

  test('submitting plan change calls BFF and closes modal', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    await page.getByTestId('change-plan-btn').click();
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });

    // Wait for plans to load
    const planSelect = page.getByTestId('plan-select');
    await expect(planSelect).toBeVisible({ timeout: BFF_TIMEOUT });

    // Select the first non-placeholder option
    const options = planSelect.locator('option:not([disabled])');
    const firstOptionValue = await options.first().getAttribute('value');
    expect(firstOptionValue).toBeTruthy();
    await planSelect.selectOption(firstOptionValue!);

    // Effective date already defaults to today — submit
    const [assignRes] = await Promise.all([
      page.waitForResponse(
        (res) => res.url().includes('/plan-assignment') && res.request().method() === 'POST',
        { timeout: BFF_TIMEOUT },
      ),
      page.getByTestId('confirm-plan-change-btn').click(),
    ]);

    expect(assignRes.status()).toBe(200);
    await expect(page.getByRole('dialog')).not.toBeVisible({ timeout: 5000 });
  });

  test('shows current plan name in plan section', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    // Plan section shows current plan
    await expect(page.getByTestId('current-plan-name')).toBeVisible();
    const planText = await page.getByTestId('current-plan-name').textContent();
    expect(planText).toBeTruthy();
  });

  test('current plan label shown inside modal', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await page.getByTestId('tab-settings').click();
    await expect(page.getByTestId('settings-tab')).toBeVisible({ timeout: BFF_TIMEOUT });

    await page.getByTestId('change-plan-btn').click();
    await expect(page.getByRole('dialog')).toBeVisible({ timeout: 5000 });

    // Modal should show the current plan
    await expect(page.getByText('Current plan:')).toBeVisible({ timeout: 5000 });
  });
});
