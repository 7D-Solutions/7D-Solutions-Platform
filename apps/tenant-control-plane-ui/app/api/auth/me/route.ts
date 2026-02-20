// ============================================================
// GET /api/auth/me
// Returns the current user's claims from the httpOnly cookie.
// Used by client code to get user info without re-reading JWT.
// Includes actor_type and support_tenant_id when a support
// session is active (separate httpOnly cookie).
// ============================================================
import { NextResponse } from 'next/server';
import { cookies } from 'next/headers';
import { decodeJwt } from 'jose';
import { getStaffClaims, guardPlatformAdmin } from '@/lib/server/auth';
import { SUPPORT_SESSION_COOKIE_NAME } from '@/lib/constants';

export async function GET() {
  const guard = await guardPlatformAdmin();
  if (guard instanceof Response) return guard;

  const claims = await getStaffClaims();
  if (!claims) {
    return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });
  }

  // Check for active support session
  const cookieStore = await cookies();
  const supportToken = cookieStore.get(SUPPORT_SESSION_COOKIE_NAME)?.value;

  let actorType: 'staff' | 'support' = 'staff';
  let supportTenantId: string | undefined;

  if (supportToken) {
    try {
      const supportClaims = decodeJwt(supportToken);
      // Verify the support token hasn't expired
      const now = Math.floor(Date.now() / 1000);
      if (!supportClaims.exp || supportClaims.exp > now) {
        actorType = 'support';
        supportTenantId = supportClaims.tenant_id as string | undefined;
      }
    } catch {
      // Invalid support token — ignore, return staff actor_type
    }
  }

  return NextResponse.json({
    sub: claims.sub,
    email: claims.email,
    roles: claims.roles,
    actor_type: actorType,
    ...(supportTenantId ? { support_tenant_id: supportTenantId } : {}),
  });
}
