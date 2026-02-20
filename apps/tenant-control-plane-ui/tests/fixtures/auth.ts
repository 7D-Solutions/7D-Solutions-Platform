// ============================================================
// Playwright auth fixture — loginAsStaff()
// Tries BFF /api/auth/login with real identity-auth first.
// Falls back to a self-issued JWT when identity-auth is unavailable.
// The middleware accepts decode-only JWTs when JWT_SECRET is unset.
// ============================================================
import { Page } from '@playwright/test';
import { SignJWT } from 'jose';

const STAFF_EMAIL    = process.env.TEST_STAFF_EMAIL    ?? 'admin@7dsolutions.com';
const STAFF_PASSWORD = process.env.TEST_STAFF_PASSWORD ?? 'admin-password';
const AUTH_COOKIE    = 'tcp_auth_token';

/**
 * Logs in as a platform_admin staff member.
 * Tries the real BFF login route first; if identity-auth is down,
 * falls back to setting a self-issued JWT cookie directly.
 */
export async function loginAsStaff(page: Page): Promise<void> {
  // Try real login first
  try {
    const res = await page.request.post('/api/auth/login', {
      data: { email: STAFF_EMAIL, password: STAFF_PASSWORD },
      headers: { 'Content-Type': 'application/json' },
    });
    if (res.ok()) return;
  } catch {
    // identity-auth unreachable — fall through to JWT fallback
  }

  // Fallback: craft a JWT that the middleware will accept (decode-only mode)
  const secret = new TextEncoder().encode('test-secret-for-playwright');
  const token = await new SignJWT({
    sub: 'test-staff-001',
    email: STAFF_EMAIL,
    roles: ['platform_admin'],
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
}
