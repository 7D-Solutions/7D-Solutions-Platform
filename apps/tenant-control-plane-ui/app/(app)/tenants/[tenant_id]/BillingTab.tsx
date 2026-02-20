// ============================================================
// BillingTab — Billing overview with charges, invoice, payment, dunning
// Each section independently renders data or "Not available".
// ============================================================
'use client';

import { useQuery } from '@tanstack/react-query';
import Link from 'next/link';
import { StatusBadge } from '@/components/ui';
import { formatCurrency, formatDate } from '@/infrastructure/utils/formatters';
import { REFETCH_INTERVAL_MS } from '@/lib/constants';
import type { BillingOverview } from '@/lib/api/types';

// ── Data fetcher ─────────────────────────────────────────────

async function fetchBillingOverview(tenantId: string): Promise<BillingOverview> {
  const res = await fetch(`/api/tenants/${encodeURIComponent(tenantId)}/billing/overview`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

// ── Component ────────────────────────────────────────────────

interface BillingTabProps {
  tenantId: string;
}

export function BillingTab({ tenantId }: BillingTabProps) {
  const billingQuery = useQuery({
    queryKey: ['tenant', tenantId, 'billing-overview'],
    queryFn: () => fetchBillingOverview(tenantId),
    refetchInterval: REFETCH_INTERVAL_MS,
  });

  if (billingQuery.isLoading) {
    return (
      <div data-testid="billing-tab">
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
          {['charges', 'last-invoice', 'outstanding', 'payment', 'dunning'].map((key) => (
            <Card key={key} title="Loading..." testId={`billing-${key}-card`}>
              <LoadingSkeleton rows={3} />
            </Card>
          ))}
        </div>
      </div>
    );
  }

  if (billingQuery.isError) {
    return (
      <div data-testid="billing-tab">
        <div
          className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-8 text-center text-[--color-danger]"
          data-testid="billing-error"
        >
          Unable to load billing information
        </div>
      </div>
    );
  }

  const data = billingQuery.data!;

  return (
    <div data-testid="billing-tab">
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
        <ChargesCard charges={data.charges} />
        <LastInvoiceCard lastInvoice={data.last_invoice} tenantId={tenantId} />
        <OutstandingCard outstanding={data.outstanding} tenantId={tenantId} />
        <PaymentStatusCard payment={data.payment_status} />
        <DunningCard dunning={data.dunning} />
      </div>
    </div>
  );
}

// ── Section Cards ────────────────────────────────────────────

function ChargesCard({ charges }: { charges: BillingOverview['charges'] }) {
  return (
    <Card title="Current Charges" testId="billing-charges-card">
      {charges.availability !== 'available' ? (
        <NotAvailable />
      ) : (
        <dl className="space-y-3">
          {charges.base_amount !== undefined && (
            <DetailRow label="Base Charge">
              {formatCurrency(charges.base_amount, charges.currency)}
            </DetailRow>
          )}
          {charges.seat_count !== undefined && (
            <DetailRow label="Seats">
              {charges.seat_count}
              {charges.seat_unit_price !== undefined && (
                <span className="text-[--color-text-secondary]">
                  {' '}@ {formatCurrency(charges.seat_unit_price, charges.currency)} each
                </span>
              )}
            </DetailRow>
          )}
          {charges.seat_total !== undefined && (
            <DetailRow label="Seat Total">
              {formatCurrency(charges.seat_total, charges.currency)}
            </DetailRow>
          )}
          {charges.metered_charges && charges.metered_charges.length > 0 && (
            <>
              <dt className="text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide pt-1">
                Metered Usage
              </dt>
              {charges.metered_charges.map((mc) => (
                <DetailRow key={mc.dimension} label={mc.dimension}>
                  {mc.quantity} units — {formatCurrency(mc.amount, charges.currency)}
                </DetailRow>
              ))}
            </>
          )}
          {charges.total !== undefined && (
            <div className="pt-2 border-t border-[--color-border-light]">
              <DetailRow label="Total">
                <span className="font-semibold">
                  {formatCurrency(charges.total, charges.currency)}
                </span>
              </DetailRow>
            </div>
          )}
          {charges.base_amount === undefined && charges.total === undefined && (
            <p className="text-sm text-[--color-text-muted]">No charges recorded</p>
          )}
        </dl>
      )}
    </Card>
  );
}

function LastInvoiceCard({
  lastInvoice,
  tenantId,
}: {
  lastInvoice: BillingOverview['last_invoice'];
  tenantId: string;
}) {
  return (
    <Card title="Last Invoice" testId="billing-last-invoice-card">
      {lastInvoice.availability !== 'available' ? (
        <NotAvailable />
      ) : !lastInvoice.id ? (
        <p className="text-sm text-[--color-text-muted]">No invoices issued yet</p>
      ) : (
        <dl className="space-y-3">
          <DetailRow label="Invoice">
            {lastInvoice.number ?? lastInvoice.id}
          </DetailRow>
          {lastInvoice.status && (
            <DetailRow label="Status">
              <StatusBadge status={lastInvoice.status} />
            </DetailRow>
          )}
          <DetailRow label="Total">
            {formatCurrency(lastInvoice.total, lastInvoice.currency)}
          </DetailRow>
          {lastInvoice.issued_at && (
            <DetailRow label="Issued">{formatDate(lastInvoice.issued_at)}</DetailRow>
          )}
          {lastInvoice.due_date && (
            <DetailRow label="Due">{formatDate(lastInvoice.due_date)}</DetailRow>
          )}
          <div className="pt-2">
            <Link
              href={`/tenants/${encodeURIComponent(tenantId)}/invoices`}
              className="text-sm text-[--color-primary] hover:underline"
              data-testid="view-all-invoices-link"
            >
              View all invoices
            </Link>
          </div>
        </dl>
      )}
    </Card>
  );
}

function OutstandingCard({
  outstanding,
  tenantId,
}: {
  outstanding: BillingOverview['outstanding'];
  tenantId: string;
}) {
  return (
    <Card title="Outstanding Balance" testId="billing-outstanding-card">
      {outstanding.availability !== 'available' ? (
        <NotAvailable />
      ) : (
        <dl className="space-y-3">
          <DetailRow label="Total Due">
            <span className={outstanding.total_due && outstanding.total_due > 0 ? 'text-[--color-danger]' : ''}>
              {formatCurrency(outstanding.total_due ?? 0, outstanding.currency)}
            </span>
          </DetailRow>
          <DetailRow label="Overdue Invoices">
            {outstanding.overdue_count ?? 0}
          </DetailRow>
          {outstanding.overdue_count !== undefined && outstanding.overdue_count > 0 && (
            <div className="pt-2">
              <Link
                href={`/tenants/${encodeURIComponent(tenantId)}/invoices`}
                className="text-sm text-[--color-primary] hover:underline"
              >
                View overdue invoices
              </Link>
            </div>
          )}
        </dl>
      )}
    </Card>
  );
}

function PaymentStatusCard({ payment }: { payment: BillingOverview['payment_status'] }) {
  // Map internal dunning terms to staff-facing labels per vision doc
  const statusLabel = (s?: string) => {
    if (!s) return '—';
    if (s === 'current') return 'Current';
    if (s === 'past_due' || s === 'DELINQUENT') return 'Past due';
    return s.charAt(0).toUpperCase() + s.slice(1).replace(/_/g, ' ');
  };

  return (
    <Card title="Payment Status" testId="billing-payment-card">
      {payment.availability !== 'available' ? (
        <NotAvailable />
      ) : (
        <dl className="space-y-3">
          <DetailRow label="Status">
            {payment.status ? (
              <StatusBadge status={payment.status} />
            ) : (
              statusLabel(payment.status)
            )}
          </DetailRow>
          {payment.last_payment_at && (
            <DetailRow label="Last Payment">
              {formatCurrency(payment.last_payment_amount, payment.currency)}
              <span className="text-[--color-text-secondary]">
                {' '}on {formatDate(payment.last_payment_at)}
              </span>
            </DetailRow>
          )}
          {!payment.status && !payment.last_payment_at && (
            <p className="text-sm text-[--color-text-muted]">No payment information</p>
          )}
        </dl>
      )}
    </Card>
  );
}

function DunningCard({ dunning }: { dunning: BillingOverview['dunning'] }) {
  // Map internal dunning states to staff-facing labels
  const dunningLabel = (s?: string) => {
    if (!s || s === 'none') return 'None';
    if (s === 'active') return 'Active';
    if (s === 'exhausted') return 'Exhausted';
    return s.charAt(0).toUpperCase() + s.slice(1).replace(/_/g, ' ');
  };

  return (
    <Card title="Past-Due Recovery" testId="billing-dunning-card">
      {dunning.availability !== 'available' ? (
        <NotAvailable />
      ) : (
        <dl className="space-y-3">
          <DetailRow label="Status">{dunningLabel(dunning.state)}</DetailRow>
          {dunning.state === 'active' && (
            <>
              {dunning.current_step !== undefined && dunning.total_steps !== undefined && (
                <DetailRow label="Retry Step">
                  {dunning.current_step} of {dunning.total_steps}
                </DetailRow>
              )}
              {dunning.next_retry_at && (
                <DetailRow label="Next Retry">{formatDate(dunning.next_retry_at)}</DetailRow>
              )}
              {dunning.started_at && (
                <DetailRow label="Started">{formatDate(dunning.started_at)}</DetailRow>
              )}
            </>
          )}
          {(!dunning.state || dunning.state === 'none') && (
            <p className="text-sm text-[--color-text-muted]">No active recovery process</p>
          )}
        </dl>
      )}
    </Card>
  );
}

// ── Shared sub-components ────────────────────────────────────

function Card({
  title,
  testId,
  children,
}: {
  title: string;
  testId: string;
  children: React.ReactNode;
}) {
  return (
    <div
      className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-5"
      data-testid={testId}
    >
      <h2 className="text-sm font-semibold text-[--color-text-primary] mb-3 pb-2 border-b border-[--color-border-light]">
        {title}
      </h2>
      {children}
    </div>
  );
}

function DetailRow({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-start justify-between">
      <dt className="text-sm text-[--color-text-secondary]">{label}</dt>
      <dd className="text-sm font-medium text-[--color-text-primary] text-right">
        {children}
      </dd>
    </div>
  );
}

function NotAvailable() {
  return (
    <div className="text-sm text-[--color-text-muted] py-2" data-testid="section-unavailable">
      Not available
    </div>
  );
}

function LoadingSkeleton({ rows }: { rows: number }) {
  return (
    <div className="space-y-3">
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="flex justify-between">
          <div className="h-4 w-20 bg-[--color-bg-secondary] rounded animate-pulse" />
          <div className="h-4 w-28 bg-[--color-bg-secondary] rounded animate-pulse" />
        </div>
      ))}
    </div>
  );
}
