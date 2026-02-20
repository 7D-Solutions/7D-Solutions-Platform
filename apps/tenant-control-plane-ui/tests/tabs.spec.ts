// ============================================================
// Tabs E2E — preview, promote, dirty tracking, close confirmation
// Verification: npx playwright test -g "Tabs|Dirty"
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';

test.describe('Tabs', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
  });

  test('tab bar is visible on tenants page', async ({ page }) => {
    await page.goto('/tenants');
    await expect(page.getByTestId('tab-bar')).toBeVisible();
    // Home tab should always be present
    await expect(page.getByTestId('tab-home')).toBeVisible();
  });

  test('home tab shows "Tenants" and is not closeable', async ({ page }) => {
    await page.goto('/tenants');
    const homeTab = page.getByTestId('tab-home');
    await expect(homeTab).toBeVisible();
    await expect(homeTab).toContainText('Tenants');
    // Home tab should not have a close button
    await expect(page.getByTestId('close-tab-home')).not.toBeVisible();
  });

  test('home tab is active by default', async ({ page }) => {
    await page.goto('/tenants');
    const homeTab = page.getByTestId('tab-home');
    await expect(homeTab).toHaveAttribute('aria-selected', 'true');
  });
});

test.describe('Dirty close confirmation', () => {
  test.beforeEach(async ({ page }) => {
    await loginAsStaff(page);
  });

  test('closing a non-dirty tab does not show confirmation', async ({ page }) => {
    await page.goto('/tenants');

    // Inject a non-dirty tab via the store
    await page.evaluate(() => {
      const store = JSON.parse(localStorage.getItem('tcp-tab-storage') || '{}');
      const newTab = {
        id: 'test-clean',
        title: 'Clean Tab',
        route: '/test-clean',
        closeable: true,
        isPreview: false,
        isDirty: false,
      };
      store.state = store.state || {};
      store.state.tabs = [...(store.state.tabs || []), newTab];
      store.state.activeTabId = 'test-clean';
      localStorage.setItem('tcp-tab-storage', JSON.stringify(store));
    });
    await page.reload();
    await loginAsStaff(page);
    await page.goto('/tenants');

    // Wait for our injected tab
    const tab = page.getByTestId('tab-test-clean');
    await expect(tab).toBeVisible();

    // Close it
    await page.getByTestId('close-tab-test-clean').click();

    // No confirmation modal should appear
    await expect(page.getByTestId('dirty-confirm')).not.toBeVisible();
    // Tab should be gone
    await expect(tab).not.toBeVisible();
  });

  test('closing a dirty tab shows confirmation modal', async ({ page }) => {
    await page.goto('/tenants');

    // Inject a dirty tab via the store
    await page.evaluate(() => {
      const store = JSON.parse(localStorage.getItem('tcp-tab-storage') || '{}');
      const dirtyTab = {
        id: 'test-dirty',
        title: 'Dirty Tab',
        route: '/test-dirty',
        closeable: true,
        isPreview: false,
        isDirty: true,
      };
      store.state = store.state || {};
      store.state.tabs = [...(store.state.tabs || []), dirtyTab];
      store.state.activeTabId = 'test-dirty';
      localStorage.setItem('tcp-tab-storage', JSON.stringify(store));
    });
    await page.reload();
    await loginAsStaff(page);
    await page.goto('/tenants');

    const tab = page.getByTestId('tab-test-dirty');
    await expect(tab).toBeVisible();

    // Should show dirty indicator
    await expect(page.getByTestId('dirty-indicator-test-dirty')).toBeVisible();

    // Try to close it
    await page.getByTestId('close-tab-test-dirty').click();

    // Confirmation modal should appear
    await expect(page.getByTestId('dirty-confirm')).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Unsaved Changes' })).toBeVisible();
    await expect(page.getByText(/Dirty Tab.+has unsaved changes/)).toBeVisible();
  });

  test('cancelling dirty close keeps the tab open', async ({ page }) => {
    await page.goto('/tenants');

    await page.evaluate(() => {
      const store = JSON.parse(localStorage.getItem('tcp-tab-storage') || '{}');
      const dirtyTab = {
        id: 'test-dirty-cancel',
        title: 'Dirty Cancel Tab',
        route: '/test-dirty-cancel',
        closeable: true,
        isPreview: false,
        isDirty: true,
      };
      store.state = store.state || {};
      store.state.tabs = [...(store.state.tabs || []), dirtyTab];
      store.state.activeTabId = 'test-dirty-cancel';
      localStorage.setItem('tcp-tab-storage', JSON.stringify(store));
    });
    await page.reload();
    await loginAsStaff(page);
    await page.goto('/tenants');

    const tab = page.getByTestId('tab-test-dirty-cancel');
    await expect(tab).toBeVisible();

    // Close button click
    await page.getByTestId('close-tab-test-dirty-cancel').click();
    await expect(page.getByTestId('dirty-confirm')).toBeVisible();

    // Click Cancel
    await page.getByTestId('dirty-cancel').click();

    // Modal should close, tab still present
    await expect(page.getByTestId('dirty-confirm')).not.toBeVisible();
    await expect(tab).toBeVisible();
  });

  test('confirming dirty close removes the tab', async ({ page }) => {
    await page.goto('/tenants');

    await page.evaluate(() => {
      const store = JSON.parse(localStorage.getItem('tcp-tab-storage') || '{}');
      const dirtyTab = {
        id: 'test-dirty-confirm',
        title: 'Dirty Confirm Tab',
        route: '/test-dirty-confirm',
        closeable: true,
        isPreview: false,
        isDirty: true,
      };
      store.state = store.state || {};
      store.state.tabs = [...(store.state.tabs || []), dirtyTab];
      store.state.activeTabId = 'test-dirty-confirm';
      localStorage.setItem('tcp-tab-storage', JSON.stringify(store));
    });
    await page.reload();
    await loginAsStaff(page);
    await page.goto('/tenants');

    const tab = page.getByTestId('tab-test-dirty-confirm');
    await expect(tab).toBeVisible();

    // Close button click
    await page.getByTestId('close-tab-test-dirty-confirm').click();
    await expect(page.getByTestId('dirty-confirm')).toBeVisible();

    // Confirm close
    await page.getByTestId('dirty-confirm').click();

    // Tab should be removed
    await expect(tab).not.toBeVisible();
    await expect(page.getByTestId('dirty-confirm')).not.toBeVisible();
  });

  test('preview tab has italic text styling', async ({ page }) => {
    await page.goto('/tenants');

    await page.evaluate(() => {
      const store = JSON.parse(localStorage.getItem('tcp-tab-storage') || '{}');
      const previewTab = {
        id: 'test-preview',
        title: 'Preview Tab',
        route: '/test-preview',
        closeable: true,
        isPreview: true,
        isDirty: false,
      };
      store.state = store.state || {};
      store.state.tabs = [...(store.state.tabs || []), previewTab];
      localStorage.setItem('tcp-tab-storage', JSON.stringify(store));
    });
    await page.reload();
    await loginAsStaff(page);
    await page.goto('/tenants');

    const tab = page.getByTestId('tab-test-preview');
    await expect(tab).toBeVisible();
    await expect(tab).toHaveAttribute('data-tab-preview', 'true');
  });
});
