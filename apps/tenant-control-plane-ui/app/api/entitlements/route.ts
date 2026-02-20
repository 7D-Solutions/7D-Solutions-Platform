// ============================================================
// GET /api/entitlements — BFF proxy to TTP entitlement catalog
// Forwards query params: search, value_type, status, page, page_size
// Auth: requires platform_admin JWT in httpOnly cookie
// Falls back to seed data when TTP is unavailable.
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { TTP_BASE_URL } from '@/lib/constants';
import { EntitlementListResponseSchema } from '@/lib/api/types';
import type { EntitlementListResponse, EntitlementSummary } from '@/lib/api/types';

// Seed entitlements returned when TTP is unavailable
const SEED_ENTITLEMENTS: EntitlementSummary[] = [
  {
    id: 'ent-max-users',
    key: 'max_users',
    label: 'Maximum Users',
    value_type: 'number',
    default_value: 10,
    status: 'active',
    created_at: '2026-01-01T00:00:00Z',
  },
  {
    id: 'ent-api-access',
    key: 'api_access',
    label: 'API Access',
    value_type: 'boolean',
    default_value: false,
    status: 'active',
    created_at: '2026-01-01T00:00:00Z',
  },
  {
    id: 'ent-storage-gb',
    key: 'storage_gb',
    label: 'Storage (GB)',
    value_type: 'number',
    default_value: 5,
    status: 'active',
    created_at: '2026-01-05T00:00:00Z',
  },
  {
    id: 'ent-sso',
    key: 'sso_enabled',
    label: 'Single Sign-On',
    value_type: 'boolean',
    default_value: false,
    status: 'active',
    created_at: '2026-01-10T00:00:00Z',
  },
  {
    id: 'ent-custom-domain',
    key: 'custom_domain',
    label: 'Custom Domain',
    value_type: 'boolean',
    default_value: false,
    status: 'draft',
    created_at: '2026-01-15T00:00:00Z',
  },
  {
    id: 'ent-support-tier',
    key: 'support_tier',
    label: 'Support Tier',
    value_type: 'string',
    default_value: 'basic',
    status: 'active',
    created_at: '2026-01-20T00:00:00Z',
  },
];

export async function GET(request: NextRequest) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { searchParams } = request.nextUrl;
  const search = searchParams.get('search') ?? '';
  const valueType = searchParams.get('value_type') ?? '';
  const status = searchParams.get('status') ?? '';
  const page = parseInt(searchParams.get('page') ?? '1', 10);
  const pageSize = parseInt(searchParams.get('page_size') ?? '25', 10);

  // Build upstream query params
  const upstreamParams = new URLSearchParams();
  if (search) upstreamParams.set('search', search);
  if (valueType) upstreamParams.set('value_type', valueType);
  if (status) upstreamParams.set('status', status);
  upstreamParams.set('page', String(page));
  upstreamParams.set('page_size', String(pageSize));

  try {
    const upstreamUrl = `${TTP_BASE_URL}/api/ttp/entitlements?${upstreamParams}`;
    const res = await fetch(upstreamUrl, {
      headers: { 'Content-Type': 'application/json' },
      signal: AbortSignal.timeout(5000),
    });

    if (!res.ok) {
      return NextResponse.json(
        { error: `Upstream error: ${res.status}` },
        { status: res.status >= 500 ? 502 : res.status },
      );
    }

    const raw = await res.json();
    const parsed = EntitlementListResponseSchema.safeParse(raw);
    if (parsed.success) {
      return NextResponse.json(parsed.data);
    }

    // Upstream returned data in unexpected shape — pass through best-effort
    return NextResponse.json(raw);
  } catch {
    // TTP unavailable — return seed data so the UI still renders
    let filtered = SEED_ENTITLEMENTS;

    if (search) {
      const q = search.toLowerCase();
      filtered = filtered.filter(
        (e) =>
          e.key.toLowerCase().includes(q) ||
          e.label.toLowerCase().includes(q),
      );
    }
    if (valueType) {
      filtered = filtered.filter((e) => e.value_type === valueType);
    }
    if (status) {
      filtered = filtered.filter((e) => e.status === status);
    }

    const total = filtered.length;
    const start = (page - 1) * pageSize;
    const paged = filtered.slice(start, start + pageSize);

    const fallback: EntitlementListResponse = {
      entitlements: paged,
      total,
      page,
      page_size: pageSize,
    };
    return NextResponse.json(fallback);
  }
}
