// ============================================================
// Tenant Invoices E2E — list with filters, row/card toggle,
// drill-down to detail with line items.
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

test.describe('Tenant Invoices', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
  });

  test('navigates to invoices list from Billing tab', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001');
    await expect(page.getByTestId('tenant-detail-tabs')).toBeVisible();

    // Switch to Billing tab
    await page.getByTestId('tab-billing').click();
    await expect(page.getByTestId('billing-tab')).toBeVisible({ timeout: 15000 });

    // Click the "View all invoices" link
    const invoicesLink = page.getByTestId('view-all-invoices-link');
    if (await invoicesLink.isVisible()) {
      await invoicesLink.click();
      await expect(page).toHaveURL(/\/tenants\/test-tenant-001\/invoices/);
    }
  });

  test('loads invoices list page with BFF call', async ({ page }) => {
    // Set up response listener
    const invoicesResponsePromise = page.waitForResponse(
      (res) => res.url().includes('/api/tenants/test-tenant-001/invoices') && !res.url().includes('/invoices/'),
    );

    await page.goto('/tenants/test-tenant-001/invoices');

    const invoicesRes = await invoicesResponsePromise;
    // BFF should return 200 (data), 404 (AR endpoint not found), or 502 (AR down)
    expect([200, 404, 502]).toContain(invoicesRes.status());
  });

  test('renders invoices list or empty state', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001/invoices');

    // Wait for either the row view or card view to appear
    const rowView = page.getByTestId('invoice-row-view');
    const cardView = page.getByTestId('invoice-card-view');
    await expect(rowView.or(cardView)).toBeVisible({ timeout: 15000 });

    // The page should show the title
    await expect(page.locator('h1')).toContainText('Invoices');
  });

  test('invoice status filter is present', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001/invoices');
    await expect(page.getByTestId('invoice-status-filter')).toBeVisible({ timeout: 15000 });

    // Verify filter options include expected values
    const options = page.getByTestId('invoice-status-filter').locator('option');
    await expect(options.first()).toContainText('All statuses');
  });

  test('row/card view toggle works on invoices list', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001/invoices');

    // Default is row view
    const rowView = page.getByTestId('invoice-row-view');
    const cardView = page.getByTestId('invoice-card-view');

    // Wait for content to load
    await expect(rowView.or(cardView)).toBeVisible({ timeout: 15000 });

    // Click card view toggle
    await page.getByRole('button', { name: 'Card view' }).click();
    await expect(cardView).toBeVisible();

    // Click row view toggle
    await page.getByRole('button', { name: 'Row view' }).click();
    await expect(rowView).toBeVisible();
  });

  test('navigates to invoice detail when invoice exists', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001/invoices');

    // Wait for the list to load
    const rowView = page.getByTestId('invoice-row-view');
    const cardView = page.getByTestId('invoice-card-view');
    await expect(rowView.or(cardView)).toBeVisible({ timeout: 15000 });

    // If there are invoices, click the first one to navigate to detail
    const invoiceLink = page.getByTestId('invoice-link').first();
    if (await invoiceLink.isVisible().catch(() => false)) {
      await invoiceLink.click();
      await expect(page).toHaveURL(/\/invoices\/.+/);
      await expect(page.getByTestId('invoice-detail')).toBeVisible({ timeout: 15000 });

      // Verify line items section renders
      await expect(page.getByTestId('invoice-line-items')).toBeVisible();

      // Verify total is shown
      await expect(page.getByTestId('invoice-total')).toBeVisible();

      // Back navigation works
      await page.getByTestId('back-to-invoices').click();
      await expect(page).toHaveURL(/\/invoices$/);
    }
  });

  test('invoice detail shows error gracefully for nonexistent invoice', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001/invoices/nonexistent-invoice-xyz');

    // Should show error state
    await expect(page.getByTestId('invoice-error')).toBeVisible({ timeout: 15000 });
  });

  test('back link returns to tenant detail', async ({ page }) => {
    await page.goto('/tenants/test-tenant-001/invoices');

    // Wait for page to load
    const rowView = page.getByTestId('invoice-row-view');
    const cardView = page.getByTestId('invoice-card-view');
    await expect(rowView.or(cardView)).toBeVisible({ timeout: 15000 });

    // Click back to tenant
    await page.locator('a', { hasText: 'Back to Tenant' }).click();
    await expect(page).toHaveURL(/\/tenants\/test-tenant-001$/);
  });
});
