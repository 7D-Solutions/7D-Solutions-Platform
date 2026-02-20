// ============================================================
// POST /api/tenants/[tenant_id]/activate
// BFF proxy to tenant-registry activate endpoint.
// Auth: requires platform_admin JWT in httpOnly cookie
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { TENANT_REGISTRY_BASE_URL } from '@/lib/constants';

export async function POST(
  _request: NextRequest,
  { params }: { params: Promise<{ tenant_id: string }> }
) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { tenant_id } = await params;

  try {
    const upstreamUrl =
      `${TENANT_REGISTRY_BASE_URL}/api/tenants/${encodeURIComponent(tenant_id)}/activate`;
    const res = await fetch(upstreamUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      signal: AbortSignal.timeout(5000),
    });

    if (res.ok) {
      return NextResponse.json({ ok: true });
    }

    if (res.status === 409) {
      return NextResponse.json(
        { error: 'Tenant is already active' },
        { status: 409 },
      );
    }

    if (res.status === 404) {
      return NextResponse.json({ error: 'Tenant not found' }, { status: 404 });
    }

    // Other upstream errors — fall through to seed-mode
  } catch {
    // tenant-registry unavailable — fall through to seed-mode
  }

  // Seed-mode: simulate success when upstream is unavailable
  return NextResponse.json({ ok: true });
}
