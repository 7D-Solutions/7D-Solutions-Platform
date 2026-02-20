// ============================================================
// GET /api/audit — BFF proxy to audit service list endpoint
// Forwards query params: actor, action, tenant_id, date_from, date_to, page, page_size
// Auth: requires platform_admin JWT in httpOnly cookie
// Enforces max page size server-side to protect against unbounded queries.
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { AUDIT_SERVICE_BASE_URL, AUDIT_MAX_PAGE_SIZE } from '@/lib/constants';
import { AuditListResponseSchema } from '@/lib/api/types';
import type { AuditListResponse } from '@/lib/api/types';

const ISO_DATE_RE = /^\d{4}-\d{2}-\d{2}$/;

export async function GET(request: NextRequest) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { searchParams } = request.nextUrl;
  const actor = searchParams.get('actor') ?? '';
  const action = searchParams.get('action') ?? '';
  const tenantId = searchParams.get('tenant_id') ?? '';
  const dateFrom = searchParams.get('date_from') ?? '';
  const dateTo = searchParams.get('date_to') ?? '';
  const page = Math.max(1, parseInt(searchParams.get('page') ?? '1', 10));
  const rawPageSize = parseInt(searchParams.get('page_size') ?? '25', 10);
  const pageSize = Math.min(Math.max(1, rawPageSize), AUDIT_MAX_PAGE_SIZE);

  // Validate date params if provided
  if (dateFrom && !ISO_DATE_RE.test(dateFrom)) {
    return NextResponse.json({ error: 'Invalid date_from format (expected YYYY-MM-DD)' }, { status: 400 });
  }
  if (dateTo && !ISO_DATE_RE.test(dateTo)) {
    return NextResponse.json({ error: 'Invalid date_to format (expected YYYY-MM-DD)' }, { status: 400 });
  }

  // Build upstream query params
  const upstreamParams = new URLSearchParams();
  if (actor) upstreamParams.set('actor', actor);
  if (action) upstreamParams.set('action', action);
  if (tenantId) upstreamParams.set('tenant_id', tenantId);
  if (dateFrom) upstreamParams.set('date_from', dateFrom);
  if (dateTo) upstreamParams.set('date_to', dateTo);
  upstreamParams.set('page', String(page));
  upstreamParams.set('page_size', String(pageSize));

  try {
    const upstreamUrl = `${AUDIT_SERVICE_BASE_URL}/api/audit?${upstreamParams}`;
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
    const parsed = AuditListResponseSchema.safeParse(raw);
    if (parsed.success) {
      return NextResponse.json(parsed.data);
    }

    // Upstream returned data but in unexpected shape — pass through best-effort
    return NextResponse.json(raw);
  } catch {
    // Audit service unavailable — return empty list so the UI still renders
    const fallback: AuditListResponse = {
      events: [],
      total: 0,
      page,
      page_size: pageSize,
    };
    return NextResponse.json(fallback);
  }
}
