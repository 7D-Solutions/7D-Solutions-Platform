// ============================================================
// GET /api/tenants/[tenant_id]/billing/overview — BFF aggregation
// Aggregates billing data from TTP (charges/usage) and AR (invoices,
// payments, dunning) into a single DTO with per-section availability.
// Auth: requires platform_admin JWT in httpOnly cookie
// ============================================================
import { NextRequest, NextResponse } from 'next/server';
import { guardPlatformAdmin } from '@/lib/server/auth';
import { TTP_BASE_URL, AR_BASE_URL } from '@/lib/constants';
import type { BillingOverview } from '@/lib/api/types';

type SectionResult<T> = { available: true; data: T } | { available: false };

async function fetchSection<T>(url: string, timeout = 5000): Promise<SectionResult<T>> {
  try {
    const res = await fetch(url, {
      headers: { 'Content-Type': 'application/json' },
      signal: AbortSignal.timeout(timeout),
    });
    if (!res.ok) return { available: false };
    const data = await res.json();
    return { available: true, data: data as T };
  } catch {
    return { available: false };
  }
}

// ── Upstream response shapes (loose — we cherry-pick what we need) ──

interface TtpCharges {
  base_amount?: number;
  seat_count?: number;
  seat_unit_price?: number;
  seat_total?: number;
  metered_charges?: Array<{ dimension: string; quantity: number; amount: number }>;
  total?: number;
  currency?: string;
}

interface ArInvoice {
  id: string;
  number?: string;
  issued_at?: string;
  due_date?: string;
  total?: number;
  status?: string;
  currency?: string;
}

interface ArInvoiceList {
  invoices: ArInvoice[];
  total: number;
}

interface ArPaymentStatus {
  status?: string;
  last_payment_at?: string;
  last_payment_amount?: number;
  currency?: string;
}

interface ArDunning {
  state?: string;
  current_step?: number;
  total_steps?: number;
  next_retry_at?: string;
  started_at?: string;
}

export async function GET(
  _request: NextRequest,
  { params }: { params: Promise<{ tenant_id: string }> }
) {
  const auth = await guardPlatformAdmin();
  if (auth instanceof Response) return auth;

  const { tenant_id } = await params;
  const enc = encodeURIComponent(tenant_id);

  // Fire all upstream requests in parallel
  const [chargesResult, invoicesResult, paymentResult, dunningResult] = await Promise.all([
    fetchSection<TtpCharges>(`${TTP_BASE_URL}/api/ttp/tenants/${enc}/charges`),
    fetchSection<ArInvoiceList>(`${AR_BASE_URL}/api/ar/tenants/${enc}/invoices?page_size=1&sort=issued_at:desc`),
    fetchSection<ArPaymentStatus>(`${AR_BASE_URL}/api/ar/tenants/${enc}/payment-status`),
    fetchSection<ArDunning>(`${AR_BASE_URL}/api/ar/tenants/${enc}/dunning`),
  ]);

  // Build charges section
  const charges: BillingOverview['charges'] = chargesResult.available
    ? {
        availability: 'available',
        base_amount: chargesResult.data.base_amount,
        seat_count: chargesResult.data.seat_count,
        seat_unit_price: chargesResult.data.seat_unit_price,
        seat_total: chargesResult.data.seat_total,
        metered_charges: chargesResult.data.metered_charges,
        total: chargesResult.data.total,
        currency: chargesResult.data.currency,
      }
    : { availability: 'unavailable' };

  // Build last invoice section
  let lastInvoice: BillingOverview['last_invoice'];
  if (invoicesResult.available && invoicesResult.data.invoices.length > 0) {
    const inv = invoicesResult.data.invoices[0];
    lastInvoice = {
      availability: 'available',
      id: inv.id,
      number: inv.number,
      issued_at: inv.issued_at,
      due_date: inv.due_date,
      total: inv.total,
      status: inv.status,
      currency: inv.currency,
    };
  } else if (invoicesResult.available) {
    lastInvoice = { availability: 'available' }; // No invoices yet
  } else {
    lastInvoice = { availability: 'unavailable' };
  }

  // Build outstanding section from invoices data
  let outstanding: BillingOverview['outstanding'];
  if (invoicesResult.available) {
    const overdueInvoices = invoicesResult.data.invoices.filter(
      (i) => i.status === 'overdue' || i.status === 'past_due'
    );
    outstanding = {
      availability: 'available',
      total_due: overdueInvoices.reduce((sum, i) => sum + (i.total ?? 0), 0),
      overdue_count: overdueInvoices.length,
      currency: invoicesResult.data.invoices[0]?.currency,
    };
  } else {
    outstanding = { availability: 'unavailable' };
  }

  // Build payment status section
  const paymentStatus: BillingOverview['payment_status'] = paymentResult.available
    ? {
        availability: 'available',
        status: paymentResult.data.status,
        last_payment_at: paymentResult.data.last_payment_at,
        last_payment_amount: paymentResult.data.last_payment_amount,
        currency: paymentResult.data.currency,
      }
    : { availability: 'unavailable' };

  // Build dunning section
  const dunning: BillingOverview['dunning'] = dunningResult.available
    ? {
        availability: 'available',
        state: dunningResult.data.state,
        current_step: dunningResult.data.current_step,
        total_steps: dunningResult.data.total_steps,
        next_retry_at: dunningResult.data.next_retry_at,
        started_at: dunningResult.data.started_at,
      }
    : { availability: 'unavailable' };

  const overview: BillingOverview = {
    charges,
    last_invoice: lastInvoice,
    outstanding,
    payment_status: paymentStatus,
    dunning,
  };

  return NextResponse.json(overview);
}
