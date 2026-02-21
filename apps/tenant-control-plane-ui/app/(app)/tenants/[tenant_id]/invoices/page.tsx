// ============================================================
// /app/tenants/[tenant_id]/invoices — Tenant invoice list
// Row/card toggle, column manager, status/date filters, pagination.
// Navigation: list → detail via row click or card click.
// ============================================================
'use client';
import { useParams, useRouter } from 'next/navigation';
import { useQuery } from '@tanstack/react-query';
import Link from 'next/link';
import { ArrowLeft, X } from 'lucide-react';
import { Button, ViewToggle, DataTable, StatusBadge, Pagination } from '@/components/ui';
import { usePersistedView } from '@/infrastructure/hooks/usePersistedView';
import { useColumnManager } from '@/infrastructure/hooks/useColumnManager';
import { usePagination } from '@/infrastructure/hooks/usePagination';
import { useFilterStore } from '@/infrastructure/state/useFilterStore';
import { formatCurrency, formatDate } from '@/infrastructure/utils/formatters';
import { REFETCH_INTERVAL_MS } from '@/lib/constants';
import type { InvoiceListResponse, InvoiceSummary } from '@/lib/api/types';
import { DEFAULT_INVOICE_FILTERS, INVOICE_STATUS_OPTIONS } from '@/lib/api/types';
import type { Column } from '@/infrastructure/hooks/useColumnManager';

// ── Column definitions ──────────────────────────────────────

const DEFAULT_COLUMNS: Column[] = [
  { id: 'number',    label: 'Invoice',   visible: true, locked: true },
  { id: 'status',    label: 'Status',    visible: true },
  { id: 'total',     label: 'Total',     visible: true, align: 'right' },
  { id: 'issued_at', label: 'Issued',    visible: true },
  { id: 'due_date',  label: 'Due',       visible: true },
  { id: 'paid_at',   label: 'Paid',      visible: false },
];

// ── Data fetcher ────────────────────────────────────────────

async function fetchInvoices(
  tenantId: string,
  params: { status: string; date_from: string; date_to: string; page: number; pageSize: number },
): Promise<InvoiceListResponse> {
  const qp = new URLSearchParams();
  if (params.status) qp.set('status', params.status);
  if (params.date_from) qp.set('date_from', params.date_from);
  if (params.date_to) qp.set('date_to', params.date_to);
  qp.set('page', String(params.page));
  qp.set('page_size', String(params.pageSize));

  const res = await fetch(
    `/api/tenants/${encodeURIComponent(tenantId)}/invoices?${qp}`,
  );
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

// ── Page component ──────────────────────────────────────────

export default function InvoicesListPage() {
  const { tenant_id } = useParams<{ tenant_id: string }>();
  const router = useRouter();
  const { viewMode, setViewMode } = usePersistedView('invoices');
  const columnManager = useColumnManager('invoice-list', DEFAULT_COLUMNS);

  const { filters, setFilter, clearFilters, hasActiveFilters } = useFilterStore(
    'invoice-list',
    DEFAULT_INVOICE_FILTERS,
  );

  const pagination = usePagination({ totalCount: 0 });

  const { data, isLoading, isError } = useQuery({
    queryKey: [
      'tenant', tenant_id, 'invoices',
      filters.status, filters.date_from, filters.date_to,
      pagination.page, pagination.pageSize,
    ],
    queryFn: () =>
      fetchInvoices(tenant_id, {
        status: filters.status,
        date_from: filters.date_from,
        date_to: filters.date_to,
        page: pagination.page,
        pageSize: pagination.pageSize,
      }),
    refetchInterval: REFETCH_INTERVAL_MS,
  });

  const invoices: InvoiceSummary[] = data?.invoices ?? [];
  const totalCount = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(totalCount / pagination.pageSize));
  const hasNextPage = pagination.page < totalPages;
  const hasPrevPage = pagination.page > 1;

  const navigateToDetail = (invoiceId: string) => {
    router.push(`/tenants/${encodeURIComponent(tenant_id)}/invoices/${encodeURIComponent(invoiceId)}`);
  };

  const TABLE_COLUMNS: Array<{
    id: string;
    header: string;
    accessor: keyof InvoiceSummary | ((row: InvoiceSummary) => React.ReactNode);
    align?: 'left' | 'right' | 'center';
  }> = [
    {
      id: 'number',
      header: 'Invoice',
      accessor: (row) => (
        <a
          href={`/tenants/${encodeURIComponent(tenant_id)}/invoices/${encodeURIComponent(row.id)}`}
          className="text-[--color-primary] hover:underline font-medium"
          data-testid="invoice-link"
        >
          {row.number ?? row.id}
        </a>
      ),
    },
    {
      id: 'status',
      header: 'Status',
      accessor: (row) => <StatusBadge status={row.status} />,
    },
    {
      id: 'total',
      header: 'Total',
      align: 'right',
      accessor: (row) => formatCurrency(row.total, row.currency),
    },
    {
      id: 'issued_at',
      header: 'Issued',
      accessor: (row) => formatDate(row.issued_at),
    },
    {
      id: 'due_date',
      header: 'Due',
      accessor: (row) => formatDate(row.due_date),
    },
    {
      id: 'paid_at',
      header: 'Paid',
      accessor: (row) => formatDate(row.paid_at),
    },
  ];

  return (
    <div>
      {/* Back link + header */}
      <div className="mb-4">
        <Link
          href={`/tenants/${encodeURIComponent(tenant_id)}`}
          className="inline-flex items-center gap-1 text-sm text-[--color-text-secondary] hover:text-[--color-primary] mb-2"
        >
          <ArrowLeft className="h-3.5 w-3.5" />
          Back to Tenant
        </Link>

        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-2xl font-semibold text-[--color-text-primary] mb-1">
              Invoices
            </h1>
            <p className="text-sm text-[--color-text-secondary]">
              {totalCount} {totalCount === 1 ? 'invoice' : 'invoices'}
            </p>
          </div>
          <ViewToggle value={viewMode} onChange={setViewMode} />
        </div>
      </div>

      {/* Filters bar */}
      <div className="flex flex-wrap items-end gap-3 mb-4" data-testid="invoice-filter-bar">
        <select
          value={filters.status}
          onChange={(e) => setFilter('status', e.target.value)}
          className="rounded-[--radius-default] border border-[--color-border-default] bg-[--color-bg-primary] px-3 py-2 text-sm text-[--color-text-primary]"
          data-testid="invoice-status-filter"
          aria-label="Filter by status"
        >
          {INVOICE_STATUS_OPTIONS.map((opt) => (
            <option key={opt.value} value={opt.value}>{opt.label}</option>
          ))}
        </select>

        <label className="flex flex-col text-xs text-[--color-text-secondary]">
          From
          <input
            type="date"
            value={filters.date_from}
            onChange={(e) => setFilter('date_from', e.target.value)}
            className="rounded-[--radius-default] border border-[--color-border-default] bg-[--color-bg-primary] px-3 py-2 text-sm text-[--color-text-primary]"
            data-testid="invoice-date-from"
          />
        </label>

        <label className="flex flex-col text-xs text-[--color-text-secondary]">
          To
          <input
            type="date"
            value={filters.date_to}
            onChange={(e) => setFilter('date_to', e.target.value)}
            className="rounded-[--radius-default] border border-[--color-border-default] bg-[--color-bg-primary] px-3 py-2 text-sm text-[--color-text-primary]"
            data-testid="invoice-date-to"
          />
        </label>

        {hasActiveFilters && (
          <Button
            variant="ghost"
            size="sm"
            onClick={clearFilters}
            icon={X}
            iconPosition="left"
          >
            Clear filters
          </Button>
        )}
      </div>

      {/* Error state */}
      {isError && (
        <div className="rounded-[--radius-lg] border border-[--color-danger] bg-red-50 p-4 mb-4 text-sm text-[--color-danger]">
          Failed to load invoices. The AR service may be unavailable.
        </div>
      )}

      {/* Row view */}
      {viewMode === 'row' ? (
        <div data-testid="invoice-row-view">
          <DataTable
            data={invoices as unknown as Record<string, unknown>[]}
            columns={TABLE_COLUMNS as DataTableColumn[]}
            columnManager={columnManager}
            keyField="id"
            loading={isLoading}
            emptyMessage={
              hasActiveFilters
                ? 'No invoices match your filters.'
                : 'No invoices found for this tenant.'
            }
            onRowClick={(row) => navigateToDetail(String(row.id))}
          />
        </div>
      ) : (
        /* Card view */
        <div
          className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4"
          data-testid="invoice-card-view"
        >
          {isLoading ? (
            <div className="col-span-full py-12 text-center text-[--color-text-muted]">
              Loading...
            </div>
          ) : invoices.length === 0 ? (
            <div className="col-span-full py-12 text-center text-[--color-text-muted]">
              {hasActiveFilters
                ? 'No invoices match your filters.'
                : 'No invoices found for this tenant.'}
            </div>
          ) : (
            invoices.map((inv) => (
              <div
                key={inv.id}
                className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-4 hover:border-[--color-primary] [transition:var(--transition-fast)] cursor-pointer"
                onClick={() => navigateToDetail(inv.id)}
                data-testid="invoice-card"
              >
                <div className="flex items-start justify-between mb-2">
                  <h3 className="font-semibold text-[--color-text-primary]">
                    {inv.number ?? inv.id}
                  </h3>
                  <StatusBadge status={inv.status} variant="compact" />
                </div>
                <p className="text-sm font-medium text-[--color-text-primary]">
                  {formatCurrency(inv.total, inv.currency)}
                </p>
                <div className="flex gap-4 mt-2 text-xs text-[--color-text-secondary]">
                  {inv.issued_at && <span>Issued {formatDate(inv.issued_at)}</span>}
                  {inv.due_date && <span>Due {formatDate(inv.due_date)}</span>}
                </div>
              </div>
            ))
          )}
        </div>
      )}

      {/* Pagination */}
      {totalCount > 0 && (
        <div className="mt-2 rounded-[--radius-lg] border border-[--color-border-light] overflow-hidden">
          <Pagination
            page={pagination.page}
            pageSize={pagination.pageSize}
            totalCount={totalCount}
            totalPages={totalPages}
            hasNextPage={hasNextPage}
            hasPrevPage={hasPrevPage}
            onNextPage={pagination.nextPage}
            onPrevPage={pagination.prevPage}
            onGoToPage={pagination.goToPage}
            onPageSizeChange={pagination.setPageSize}
          />
        </div>
      )}
    </div>
  );
}

// Helper type to satisfy DataTable generic constraint
type DataTableColumn = {
  id: string;
  header: string;
  accessor: string | ((row: Record<string, unknown>) => React.ReactNode);
  align?: 'left' | 'center' | 'right';
};
