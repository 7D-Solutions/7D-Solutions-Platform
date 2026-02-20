// ============================================================
// Login & Logout E2E — staff login UX and session lifecycle
// Covers: login success, login failure, logout, forbidden, redirect.
// Verification: npx playwright test -g "Login|Logout"
// ============================================================
import { test, expect } from '@playwright/test';
import { loginAsStaff } from './fixtures/auth';
import { SignJWT } from 'jose';

const AUTH_COOKIE = 'tcp_auth_token';

test.describe('Login', () => {
  test('renders login form with email and password fields', async ({ page }) => {
    await page.goto('/login');
    await expect(page.getByRole('heading', { name: /staff login/i })).toBeVisible();
    await expect(page.getByLabel(/email/i)).toBeVisible();
    await expect(page.getByLabel(/password/i)).toBeVisible();
    await expect(page.getByRole('button', { name: /sign in/i })).toBeVisible();
  });

  test('shows validation errors for empty fields', async ({ page }) => {
    await page.goto('/login');
    // Click sign in without filling fields
    await page.getByRole('button', { name: /sign in/i }).click();
    // RHF+Zod validation should show errors
    await expect(page.getByText(/email is required/i)).toBeVisible();
    await expect(page.getByText(/password is required/i)).toBeVisible();
  });

  test('shows validation error for invalid email format', async ({ page }) => {
    await page.goto('/login');
    await page.getByLabel(/email/i).fill('not-an-email');
    await page.getByLabel(/password/i).fill('password');
    await page.getByRole('button', { name: /sign in/i }).click();
    await expect(page.getByText(/valid email/i)).toBeVisible();
  });

  test('shows server error on invalid credentials', async ({ page }) => {
    await page.goto('/login');
    await page.getByLabel(/email/i).fill('wrong@example.com');
    await page.getByLabel(/password/i).fill('wrong-password');
    await page.getByRole('button', { name: /sign in/i }).click();
    // BFF returns error — should display server error
    await expect(page.getByTestId('server-error')).toBeVisible({ timeout: 10000 });
  });

  test('successful login redirects to /tenants', async ({ page }) => {
    await loginAsStaff(page);
    await page.goto('/tenants');
    await expect(page).not.toHaveURL(/\/login/);
    await expect(page.getByRole('heading', { name: 'Tenants' })).toBeVisible();
  });

  test('login stores JWT in httpOnly cookie only', async ({ page }) => {
    await loginAsStaff(page);
    // Cookie should exist (set by loginAsStaff)
    const cookies = await page.context().cookies();
    const authCookie = cookies.find((c) => c.name === AUTH_COOKIE);
    expect(authCookie).toBeDefined();
    expect(authCookie!.httpOnly).toBe(true);

    // Token should NOT be in localStorage or sessionStorage
    const localToken = await page.evaluate(() => localStorage.getItem(AUTH_COOKIE));
    const sessionToken = await page.evaluate(() => sessionStorage.getItem(AUTH_COOKIE));
    expect(localToken).toBeNull();
    expect(sessionToken).toBeNull();
  });

  test('displays session expired message when reason=expired', async ({ page }) => {
    await page.goto('/login?reason=expired');
    await expect(page.getByText(/session has expired/i)).toBeVisible();
  });

  test('unauthenticated request redirects to login with redirect param', async ({ page }) => {
    await page.goto('/tenants');
    await expect(page).toHaveURL(/\/login\?redirect=/);
  });

  test('forbidden user is redirected to /forbidden', async ({ page }) => {
    // Set a JWT with a non-admin role
    const secret = new TextEncoder().encode('test-secret');
    const token = await new SignJWT({
      sub: 'test-user-001',
      email: 'viewer@example.com',
      roles: ['viewer'],
    })
      .setProtectedHeader({ alg: 'HS256' })
      .setIssuedAt()
      .setExpirationTime('8h')
      .sign(secret);

    await page.context().addCookies([{
      name: AUTH_COOKIE,
      value: token,
      domain: 'localhost',
      path: '/',
      httpOnly: true,
      secure: false,
      sameSite: 'Lax',
    }]);

    await page.goto('/tenants');
    await expect(page).toHaveURL(/\/forbidden/);
    await expect(page.getByText(/access denied/i)).toBeVisible();
  });
});

test.describe('Logout', () => {
  test('sidebar logout clears cookie and redirects to login', async ({ page }) => {
    await loginAsStaff(page);
    await page.goto('/tenants');
    // Click sidebar logout button
    await page.getByRole('button', { name: /log out/i }).first().click();
    await expect(page).toHaveURL(/\/login/);

    // Cookie should be cleared
    const cookies = await page.context().cookies();
    const authCookie = cookies.find((c) => c.name === AUTH_COOKIE);
    expect(!authCookie || authCookie.value === '').toBe(true);
  });

  test('user menu logout clears cookie and redirects to login', async ({ page }) => {
    await loginAsStaff(page);
    await page.goto('/tenants');
    // Open user menu dropdown
    await page.getByTestId('user-menu').click();
    // Click Log out in the dropdown menu
    await page.getByRole('menuitem', { name: /log out/i }).click();
    await expect(page).toHaveURL(/\/login/);
  });

  test('after logout, protected routes redirect back to login', async ({ page }) => {
    await loginAsStaff(page);
    await page.goto('/tenants');
    // Logout
    await page.getByRole('button', { name: /log out/i }).first().click();
    await expect(page).toHaveURL(/\/login/);
    // Try to access protected route
    await page.goto('/tenants');
    await expect(page).toHaveURL(/\/login/);
  });
});
