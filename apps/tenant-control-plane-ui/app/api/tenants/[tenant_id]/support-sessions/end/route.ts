// ============================================================
// POST /api/tenants/[tenant_id]/support-sessions/end
// BFF — ends an active support session for the given tenant.
// Clears the support session httpOnly cookie.
// Auth: requires existing platform_admin JWT in httpOnly cookie
// ============================================================
import { NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { SUPPORT_SESSION_COOKIE_NAME, IDENTITY_AUTH_BASE_URL } from '@/lib/constants';
import { cookies } from 'next/headers';

export async function POST(
  _req: Request,
  { params }: { params: Promise<{ tenant_id: string }> },
) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { tenant_id } = await params;

  // Check that we have an active support session
  const cookieStore = await cookies();
  const supportToken = cookieStore.get(SUPPORT_SESSION_COOKIE_NAME)?.value;

  if (!supportToken) {
    return NextResponse.json({ error: 'No active support session' }, { status: 400 });
  }

  // Notify identity-auth to revoke/log the session end (best-effort)
  try {
    await fetch(`${IDENTITY_AUTH_BASE_URL}/auth/support-session/end`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${supportToken}`,
      },
      body: JSON.stringify({ tenant_id }),
      signal: AbortSignal.timeout(5000),
    });
  } catch {
    // Proceed even if backend is unavailable — clearing cookie is enough
  }

  const response = NextResponse.json({
    ok: true,
    actor_type: 'staff',
    tenant_id,
  });

  // Clear the support session cookie
  response.cookies.set(SUPPORT_SESSION_COOKIE_NAME, '', {
    httpOnly: true,
    secure: process.env.NODE_ENV === 'production',
    sameSite: 'lax',
    path: '/',
    maxAge: 0,
  });

  return response;
}
