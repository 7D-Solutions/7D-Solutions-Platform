// ============================================================
// GET /api/tenants/[tenant_id]/invoices — BFF proxy to AR invoice list
// Filters by tenant_id, status, date range. Supports pagination.
// Auth: requires platform_admin JWT in httpOnly cookie
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { AR_BASE_URL } from '@/lib/constants';

export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ tenant_id: string }> }
) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { tenant_id } = await params;
  const enc = encodeURIComponent(tenant_id);

  // Forward query params from the client
  const url = new URL(request.url);
  const qp = new URLSearchParams();

  const page = url.searchParams.get('page') ?? '1';
  const pageSize = url.searchParams.get('page_size') ?? '25';
  const status = url.searchParams.get('status');
  const dateFrom = url.searchParams.get('date_from');
  const dateTo = url.searchParams.get('date_to');

  qp.set('page', page);
  qp.set('page_size', pageSize);
  if (status) qp.set('status', status);
  if (dateFrom) qp.set('date_from', dateFrom);
  if (dateTo) qp.set('date_to', dateTo);

  try {
    const res = await fetch(
      `${AR_BASE_URL}/api/ar/tenants/${enc}/invoices?${qp}`,
      {
        headers: { 'Content-Type': 'application/json' },
        signal: AbortSignal.timeout(10000),
      },
    );

    if (!res.ok) {
      return NextResponse.json(
        { error: `AR service returned ${res.status}` },
        { status: res.status >= 500 ? 502 : res.status },
      );
    }

    const data = await res.json();
    return NextResponse.json(data);
  } catch {
    return NextResponse.json(
      { error: 'AR service unavailable', invoices: [], total: 0, page: 1, page_size: 25 },
      { status: 502 },
    );
  }
}
