// ============================================================
// GET /api/auth/me
// Returns the current user's claims from the httpOnly cookie.
// Used by client code to get user info without re-reading JWT.
// ============================================================
import { NextResponse } from 'next/server';
import { getStaffClaims, guardPlatformAdmin } from '@/lib/server/auth';

export async function GET() {
  const guard = await guardPlatformAdmin();
  if (guard instanceof Response) return guard;

  const claims = await getStaffClaims();
  if (!claims) {
    return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });
  }

  return NextResponse.json({
    sub: claims.sub,
    email: claims.email,
    roles: claims.roles,
  });
}
