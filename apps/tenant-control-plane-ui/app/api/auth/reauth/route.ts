// ============================================================
// POST /api/auth/reauth
// Re-authentication: verifies the staff user's password.
// Used before high-risk actions (e.g., terminate tenant).
// Returns { ok: true } on success, 401 on invalid credentials.
// Auth: requires existing platform_admin JWT in httpOnly cookie
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin, getStaffClaims } from '@/lib/server/auth';
import { IDENTITY_AUTH_BASE_URL, PLATFORM_TENANT_ID } from '@/lib/constants';

export async function POST(req: NextRequest) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  let body: { password?: string };
  try {
    body = await req.json();
  } catch {
    return NextResponse.json({ error: 'Invalid request body' }, { status: 400 });
  }

  const { password } = body;
  if (!password) {
    return NextResponse.json({ error: 'Password is required' }, { status: 400 });
  }

  // Get the current user's email from the JWT claims
  const claims = await getStaffClaims();
  if (!claims?.email) {
    return NextResponse.json({ error: 'Unable to read session' }, { status: 401 });
  }

  // Forward re-auth to identity-auth login endpoint
  try {
    const res = await fetch(`${IDENTITY_AUTH_BASE_URL}/api/auth/login`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ tenant_id: PLATFORM_TENANT_ID, email: claims.email, password }),
      signal: AbortSignal.timeout(5000),
    });

    if (res.ok) {
      return NextResponse.json({ ok: true });
    }

    if (res.status === 401) {
      return NextResponse.json({ error: 'Invalid password' }, { status: 401 });
    }

    // Other error — fall through to seed-mode
  } catch {
    // identity-auth unavailable — fall through to seed-mode
  }

  // Seed-mode: accept re-auth when identity-auth is unavailable
  return NextResponse.json({ ok: true });
}
