// ============================================================
// /app/tenants/[tenant_id]/invoices/[invoice_id] — Invoice detail
// Shows invoice header, status badge, line items table, and totals.
// ============================================================
'use client';
import { useParams } from 'next/navigation';
import { useQuery } from '@tanstack/react-query';
import Link from 'next/link';
import { ArrowLeft } from 'lucide-react';
import { StatusBadge } from '@/components/ui';
import { formatCurrency, formatDate } from '@/infrastructure/utils/formatters';
import { REFETCH_INTERVAL_MS } from '@/lib/constants';
import type { InvoiceDetail } from '@/lib/api/types';

// ── Data fetcher ────────────────────────────────────────────

async function fetchInvoiceDetail(
  tenantId: string,
  invoiceId: string,
): Promise<InvoiceDetail> {
  const res = await fetch(
    `/api/tenants/${encodeURIComponent(tenantId)}/invoices/${encodeURIComponent(invoiceId)}`,
  );
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

// ── Page component ──────────────────────────────────────────

export default function InvoiceDetailPage() {
  const { tenant_id, invoice_id } = useParams<{
    tenant_id: string;
    invoice_id: string;
  }>();

  const { data: invoice, isLoading, isError } = useQuery({
    queryKey: ['tenant', tenant_id, 'invoice', invoice_id],
    queryFn: () => fetchInvoiceDetail(tenant_id, invoice_id),
    refetchInterval: REFETCH_INTERVAL_MS,
  });

  return (
    <div>
      {/* Back link */}
      <div className="mb-4">
        <Link
          href={`/tenants/${encodeURIComponent(tenant_id)}/invoices`}
          className="inline-flex items-center gap-1 text-sm text-[--color-text-secondary] hover:text-[--color-primary] mb-2"
          data-testid="back-to-invoices"
        >
          <ArrowLeft className="h-3.5 w-3.5" />
          Back to Invoices
        </Link>
      </div>

      {isLoading && (
        <div className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-8 text-center text-[--color-text-muted]">
          Loading invoice...
        </div>
      )}

      {isError && (
        <div
          className="rounded-[--radius-lg] border border-[--color-danger] bg-red-50 p-8 text-center text-[--color-danger]"
          data-testid="invoice-error"
        >
          Unable to load invoice details. The AR service may be unavailable.
        </div>
      )}

      {invoice && (
        <div data-testid="invoice-detail">
          {/* Header */}
          <div className="flex items-center gap-3 mb-6">
            <h1 className="text-2xl font-semibold text-[--color-text-primary]">
              Invoice {invoice.number ?? invoice.id}
            </h1>
            <StatusBadge status={invoice.status} />
          </div>

          {/* Summary card */}
          <div className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-5 mb-6">
            <h2 className="text-sm font-semibold text-[--color-text-primary] mb-3 pb-2 border-b border-[--color-border-light]">
              Details
            </h2>
            <dl className="grid grid-cols-2 md:grid-cols-4 gap-4">
              <DetailItem label="Status">
                <StatusBadge status={invoice.status} />
              </DetailItem>
              <DetailItem label="Total">
                {formatCurrency(invoice.total, invoice.currency)}
              </DetailItem>
              <DetailItem label="Issued">
                {formatDate(invoice.issued_at)}
              </DetailItem>
              <DetailItem label="Due">
                {formatDate(invoice.due_date)}
              </DetailItem>
              {invoice.paid_at && (
                <DetailItem label="Paid">
                  {formatDate(invoice.paid_at)}
                </DetailItem>
              )}
            </dl>
          </div>

          {/* Line items table */}
          <div
            className="rounded-[--radius-lg] border border-[--color-border-light] overflow-hidden"
            data-testid="invoice-line-items"
          >
            <div className="border-b border-[--color-border-light] bg-[--color-bg-secondary] px-4 py-3">
              <h2 className="text-sm font-semibold text-[--color-text-primary]">
                Line Items
              </h2>
            </div>

            <table className="w-full border-collapse text-sm">
              <thead>
                <tr className="border-b border-[--color-border-light] bg-[--color-bg-secondary]">
                  <th className="px-4 py-3 text-left text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">
                    Description
                  </th>
                  <th className="px-4 py-3 text-right text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">
                    Qty
                  </th>
                  <th className="px-4 py-3 text-right text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">
                    Unit Price
                  </th>
                  <th className="px-4 py-3 text-right text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">
                    Amount
                  </th>
                </tr>
              </thead>
              <tbody>
                {invoice.line_items.length === 0 ? (
                  <tr>
                    <td
                      colSpan={4}
                      className="py-8 text-center text-[--color-text-muted]"
                    >
                      No line items
                    </td>
                  </tr>
                ) : (
                  invoice.line_items.map((item) => (
                    <tr
                      key={item.id}
                      className="border-b border-[--color-border-light] hover:bg-[--color-bg-secondary] transition-[--transition-fast]"
                      data-testid="line-item-row"
                    >
                      <td className="px-4 py-3 text-[--color-text-primary]">
                        {item.description}
                      </td>
                      <td className="px-4 py-3 text-right text-[--color-text-primary] tabular-nums">
                        {item.quantity}
                      </td>
                      <td className="px-4 py-3 text-right text-[--color-text-primary] tabular-nums">
                        {formatCurrency(item.unit_price, invoice.currency)}
                      </td>
                      <td className="px-4 py-3 text-right text-[--color-text-primary] tabular-nums">
                        {formatCurrency(item.amount, invoice.currency)}
                      </td>
                    </tr>
                  ))
                )}
              </tbody>
            </table>

            {/* Totals footer */}
            <div className="border-t border-[--color-border-light] bg-[--color-bg-secondary] px-4 py-3 space-y-1">
              {invoice.subtotal !== undefined && (
                <div className="flex justify-between text-sm">
                  <span className="text-[--color-text-secondary]">Subtotal</span>
                  <span className="text-[--color-text-primary] tabular-nums">
                    {formatCurrency(invoice.subtotal, invoice.currency)}
                  </span>
                </div>
              )}
              {invoice.tax !== undefined && invoice.tax > 0 && (
                <div className="flex justify-between text-sm">
                  <span className="text-[--color-text-secondary]">Tax</span>
                  <span className="text-[--color-text-primary] tabular-nums">
                    {formatCurrency(invoice.tax, invoice.currency)}
                  </span>
                </div>
              )}
              <div className="flex justify-between text-sm font-semibold pt-1 border-t border-[--color-border-light]">
                <span className="text-[--color-text-primary]">Total</span>
                <span className="text-[--color-text-primary] tabular-nums" data-testid="invoice-total">
                  {formatCurrency(invoice.total, invoice.currency)}
                </span>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

// ── Shared sub-component ────────────────────────────────────

function DetailItem({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <dt className="text-xs text-[--color-text-secondary] uppercase tracking-wide mb-1">
        {label}
      </dt>
      <dd className="text-sm font-medium text-[--color-text-primary]">
        {children}
      </dd>
    </div>
  );
}
