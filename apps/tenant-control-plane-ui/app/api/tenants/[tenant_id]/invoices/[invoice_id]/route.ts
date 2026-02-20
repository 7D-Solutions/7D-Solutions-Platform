// ============================================================
// GET /api/tenants/[tenant_id]/invoices/[invoice_id] — BFF proxy to AR
// Fetches invoice detail and validates tenant context (confused deputy guard).
// Auth: requires platform_admin JWT in httpOnly cookie
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { AR_BASE_URL } from '@/lib/constants';

export async function GET(
  _request: NextRequest,
  { params }: { params: Promise<{ tenant_id: string; invoice_id: string }> }
) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { tenant_id, invoice_id } = await params;
  const encTenant = encodeURIComponent(tenant_id);
  const encInvoice = encodeURIComponent(invoice_id);

  try {
    const res = await fetch(
      `${AR_BASE_URL}/api/ar/tenants/${encTenant}/invoices/${encInvoice}`,
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

    // Confused deputy guard: verify invoice belongs to the requested tenant
    if (data.tenant_id && data.tenant_id !== tenant_id) {
      return NextResponse.json(
        { error: 'Invoice does not belong to this tenant' },
        { status: 403 },
      );
    }

    return NextResponse.json(data);
  } catch {
    return NextResponse.json(
      { error: 'AR service unavailable' },
      { status: 502 },
    );
  }
}
