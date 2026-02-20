// ============================================================
// GET /api/tenants/[tenant_id]/users — BFF proxy to identity-auth
// Returns tenant-scoped user list. Falls back to seed data when
// identity-auth is unavailable.
// Auth: requires platform_admin JWT in httpOnly cookie
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { IDENTITY_AUTH_BASE_URL } from '@/lib/constants';
import { TenantUserListResponseSchema } from '@/lib/api/types';
import type { TenantUserListResponse } from '@/lib/api/types';

export async function GET(
  _request: NextRequest,
  { params }: { params: Promise<{ tenant_id: string }> }
) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { tenant_id } = await params;

  try {
    const upstreamUrl = `${IDENTITY_AUTH_BASE_URL}/api/tenants/${encodeURIComponent(tenant_id)}/users`;
    const res = await fetch(upstreamUrl, {
      headers: { 'Content-Type': 'application/json' },
      signal: AbortSignal.timeout(5000),
    });

    if (!res.ok) {
      if (res.status === 404) {
        // Tenant has no users yet — return empty list
        return NextResponse.json({ users: [], total: 0 });
      }
      return NextResponse.json(
        { error: `Upstream error: ${res.status}` },
        { status: res.status >= 500 ? 502 : res.status },
      );
    }

    const raw = await res.json();
    const parsed = TenantUserListResponseSchema.safeParse(raw);
    if (parsed.success) {
      return NextResponse.json(parsed.data);
    }
    return NextResponse.json(raw);
  } catch {
    // identity-auth unavailable — return seed data so the UI still renders
    const fallback: TenantUserListResponse = {
      users: [
        {
          id: 'seed-user-001',
          email: 'admin@example.com',
          name: 'Admin User',
          status: 'active',
          last_seen: new Date().toISOString(),
          created_at: '2025-01-15T10:00:00Z',
        },
        {
          id: 'seed-user-002',
          email: 'viewer@example.com',
          name: 'Viewer User',
          status: 'active',
          created_at: '2025-02-01T14:30:00Z',
        },
        {
          id: 'seed-user-003',
          email: 'suspended@example.com',
          name: 'Suspended User',
          status: 'deactivated',
          created_at: '2025-01-20T09:00:00Z',
        },
      ],
      total: 3,
    };
    return NextResponse.json(fallback);
  }
}
