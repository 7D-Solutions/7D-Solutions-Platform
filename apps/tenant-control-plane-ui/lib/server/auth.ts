// ============================================================
// TCP UI — Server-only auth utilities
// IMPORTANT: This file is server-only. Never import in client components.
// ============================================================
import { cookies } from 'next/headers';
import { jwtVerify, decodeJwt } from 'jose';
import { AUTH_COOKIE_NAME, REQUIRED_ROLE } from '@/lib/constants';

export interface StaffClaims {
  sub: string;           // User ID
  email: string;
  name?: string;
  roles?: string[];
  perms?: string[];
  exp: number;
  iat: number;
}

/**
 * Read and decode the staff JWT from the httpOnly cookie.
 * Returns null if the cookie is missing or invalid.
 */
export async function getStaffToken(): Promise<string | null> {
  const cookieStore = await cookies();
  return cookieStore.get(AUTH_COOKIE_NAME)?.value ?? null;
}

/**
 * Decode the JWT claims without verifying the signature.
 * Used for reading claims after the backend has already validated the token.
 * For full verification, use verifyStaffToken.
 */
export async function getStaffClaims(): Promise<StaffClaims | null> {
  const token = await getStaffToken();
  if (!token) return null;

  try {
    const payload = decodeJwt(token);
    return payload as unknown as StaffClaims;
  } catch {
    return null;
  }
}

/**
 * Verify the JWT signature and return claims.
 * Uses the JWT_SECRET environment variable.
 */
export async function verifyStaffToken(): Promise<StaffClaims | null> {
  const token = await getStaffToken();
  if (!token) return null;

  const secret = process.env.JWT_SECRET;
  if (!secret) {
    // In development without a secret, fall back to decode-only
    return getStaffClaims();
  }

  try {
    const { payload } = await jwtVerify(
      token,
      new TextEncoder().encode(secret)
    );
    return payload as unknown as StaffClaims;
  } catch {
    return null;
  }
}

/**
 * Check if the current request has a valid staff JWT with platform_admin role.
 * Returns the claims on success, null on failure.
 */
export async function requirePlatformAdmin(): Promise<StaffClaims | null> {
  const claims = await getStaffClaims();
  if (!claims) return null;

  const hasPlatformAdmin =
    (claims.roles?.includes(REQUIRED_ROLE) ?? false) ||
    (claims.perms?.includes(REQUIRED_ROLE) ?? false) ||
    (claims.perms?.some((p) => p.startsWith('platform_admin')) ?? false);

  return hasPlatformAdmin ? claims : null;
}

/**
 * Standard 401 response for unauthenticated requests.
 */
export function unauthorized(): Response {
  return new Response(JSON.stringify({ error: 'Unauthorized' }), {
    status: 401,
    headers: { 'Content-Type': 'application/json' },
  });
}

/**
 * Standard 403 response for authenticated but unauthorized requests.
 */
export function forbidden(): Response {
  return new Response(JSON.stringify({ error: 'Forbidden — platform_admin required' }), {
    status: 403,
    headers: { 'Content-Type': 'application/json' },
  });
}

/**
 * BFF route guard — use at the top of every BFF handler.
 * Returns claims on success or a Response to return immediately on failure.
 *
 * @example
 * export async function GET() {
 *   const auth = await guardPlatformAdmin();
 *   if (auth instanceof Response) return auth;
 *   // auth is StaffClaims — proceed
 * }
 */
export async function guardPlatformAdmin(): Promise<StaffClaims | Response> {
  const token = await getStaffToken();
  if (!token) return unauthorized();

  const claims = await requirePlatformAdmin();
  if (!claims) return forbidden();

  return claims;
}
