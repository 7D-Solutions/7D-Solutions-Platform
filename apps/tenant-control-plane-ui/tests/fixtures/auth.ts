// ============================================================
// Playwright auth fixture — loginAsStaff()
// Calls the BFF /api/auth/login route with real identity-auth
// credentials from environment variables.
// ============================================================
import { Page } from '@playwright/test';

const STAFF_EMAIL    = process.env.TEST_STAFF_EMAIL    ?? 'admin@7dsolutions.com';
const STAFF_PASSWORD = process.env.TEST_STAFF_PASSWORD ?? 'admin-password';

/**
 * Logs in as a platform_admin staff member via the BFF login endpoint.
 * Sets the httpOnly auth cookie in the browser context.
 * After this call the page is ready to navigate to /app/** routes.
 */
export async function loginAsStaff(page: Page): Promise<void> {
  const res = await page.request.post('/api/auth/login', {
    data: { email: STAFF_EMAIL, password: STAFF_PASSWORD },
    headers: { 'Content-Type': 'application/json' },
  });

  if (!res.ok()) {
    const body = await res.text().catch(() => '(unreadable)');
    throw new Error(
      `loginAsStaff failed: HTTP ${res.status()} — ${body}\n` +
      `Check TEST_STAFF_EMAIL / TEST_STAFF_PASSWORD env vars and that identity-auth is running.`,
    );
  }
}
