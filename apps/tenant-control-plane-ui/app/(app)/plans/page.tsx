// ============================================================
// /app/plans — Plan catalog list with column manager,
// row/card toggle, status filter, and TanStack Query
// ============================================================
'use client';
import { useQuery } from '@tanstack/react-query';
import { Button, ViewToggle, DataTable, StatusBadge, Pagination } from '@/components/ui';
import { usePersistedView } from '@/infrastructure/hooks/usePersistedView';
import { useColumnManager } from '@/infrastructure/hooks/useColumnManager';
import { usePagination } from '@/infrastructure/hooks/usePagination';
import { useFilterStore } from '@/infrastructure/state/useFilterStore';
import { REFETCH_INTERVAL_MS } from '@/lib/constants';
import { PLAN_STATUS_OPTIONS } from '@/lib/api/types';
import type { PlanListResponse, PlanSummary } from '@/lib/api/types';
import type { Column } from '@/infrastructure/hooks/useColumnManager';

// ── Column definitions ──────────────────────────────────────

const DEFAULT_COLUMNS: Column[] = [
  { id: 'name',               label: 'Name',               visible: true, locked: true },
  { id: 'pricing_model',      label: 'Pricing Model',      visible: true },
  { id: 'included_seats',     label: 'Included Seats',     visible: true },
  { id: 'metered_dimensions', label: 'Metered Dimensions', visible: true },
  { id: 'status',             label: 'Status',             visible: true },
  { id: 'created_at',         label: 'Created',            visible: false },
];

const TABLE_COLUMNS: Array<{
  id: string;
  header: string;
  accessor: keyof PlanSummary | ((row: PlanSummary) => React.ReactNode);
}> = [
  { id: 'name',               header: 'Name',               accessor: 'name' },
  { id: 'pricing_model',      header: 'Pricing Model',      accessor: (row) => formatPricingModel(row.pricing_model) },
  { id: 'included_seats',     header: 'Included Seats',     accessor: (row) => String(row.included_seats) },
  { id: 'metered_dimensions', header: 'Metered Dimensions', accessor: (row) => formatDimensions(row.metered_dimensions) },
  { id: 'status',             header: 'Status',             accessor: (row) => <StatusBadge status={row.status} /> },
  { id: 'created_at',         header: 'Created',            accessor: (row) => formatDate(row.created_at) },
];

function formatPricingModel(model: string): string {
  const labels: Record<string, string> = {
    flat: 'Flat rate',
    per_seat: 'Per seat',
    tiered: 'Tiered',
    usage: 'Usage-based',
  };
  return labels[model] ?? model;
}

function formatDimensions(dims: string[]): string {
  if (dims.length === 0) return '—';
  return dims.map((d) => d.replace(/_/g, ' ')).join(', ');
}

function formatDate(iso?: string): string {
  if (!iso) return '—';
  try {
    return new Date(iso).toLocaleDateString('en-US', {
      month: 'short', day: 'numeric', year: 'numeric',
    });
  } catch {
    return iso;
  }
}

// ── Default filters ─────────────────────────────────────────

const DEFAULT_PLAN_FILTERS: Record<string, string> & { status: string } = {
  status: '',
};

// ── Data fetcher ────────────────────────────────────────────

async function fetchPlans(params: {
  status: string;
  page: number;
  pageSize: number;
}): Promise<PlanListResponse> {
  const qp = new URLSearchParams();
  if (params.status) qp.set('status', params.status);
  qp.set('page', String(params.page));
  qp.set('page_size', String(params.pageSize));

  const res = await fetch(`/api/plans?${qp}`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

// ── Page component ──────────────────────────────────────────

export default function PlansPage() {
  const { viewMode, setViewMode } = usePersistedView('plans');
  const columnManager = useColumnManager('plan-list', DEFAULT_COLUMNS);

  const { filters, setFilter, clearFilters, hasActiveFilters } = useFilterStore(
    'plan-list',
    DEFAULT_PLAN_FILTERS,
  );

  const pagination = usePagination({ totalCount: 0 });

  const { data, isLoading, isError } = useQuery({
    queryKey: ['plans', filters.status, pagination.page, pagination.pageSize],
    queryFn: () =>
      fetchPlans({
        status: filters.status,
        page: pagination.page,
        pageSize: pagination.pageSize,
      }),
    refetchInterval: REFETCH_INTERVAL_MS,
  });

  const plans: PlanSummary[] = data?.plans ?? [];
  const totalCount = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(totalCount / pagination.pageSize));
  const hasNextPage = pagination.page < totalPages;
  const hasPrevPage = pagination.page > 1;

  return (
    <div>
      {/* Header */}
      <div className="flex items-center justify-between mb-4">
        <div>
          <h1 className="text-2xl font-semibold text-[--color-text-primary] mb-1">Plans &amp; Pricing</h1>
          <p className="text-sm text-[--color-text-secondary]">
            {totalCount} {totalCount === 1 ? 'plan' : 'plans'}
          </p>
        </div>
        <ViewToggle value={viewMode} onChange={setViewMode} />
      </div>

      {/* Status filter bar */}
      <div
        className="flex flex-wrap items-end gap-3 mb-4"
        data-testid="plan-filter-bar"
      >
        <select
          value={filters.status}
          onChange={(e) => setFilter('status', e.target.value)}
          className="rounded-[--radius-default] border border-[--color-border-default] bg-[--color-bg-primary] px-3 py-2 text-sm text-[--color-text-primary]"
          data-testid="plan-status-filter"
          aria-label="Filter by status"
        >
          {PLAN_STATUS_OPTIONS.map((opt) => (
            <option key={opt.value} value={opt.value}>{opt.label}</option>
          ))}
        </select>

        {hasActiveFilters && (
          <Button
            variant="ghost"
            size="sm"
            onClick={clearFilters}
          >
            Clear filters
          </Button>
        )}
      </div>

      {/* Error state */}
      {isError && (
        <div className="rounded-[--radius-lg] border border-[--color-danger] bg-red-50 p-4 mb-4 text-sm text-[--color-danger]">
          Failed to load plans. The plan service may be unavailable.
        </div>
      )}

      {/* Row view — DataTable with column manager */}
      {viewMode === 'row' ? (
        <div data-testid="plan-row-view">
          <DataTable
            data={plans as unknown as Record<string, unknown>[]}
            columns={TABLE_COLUMNS as DataTableColumn[]}
            columnManager={columnManager}
            keyField="id"
            loading={isLoading}
            emptyMessage={
              hasActiveFilters
                ? 'No plans match your filters.'
                : 'No plans found.'
            }
          />
        </div>
      ) : (
        /* Card view */
        <div
          className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4"
          data-testid="plan-card-view"
        >
          {isLoading ? (
            <div className="col-span-full py-12 text-center text-[--color-text-muted]">
              Loading...
            </div>
          ) : plans.length === 0 ? (
            <div className="col-span-full py-12 text-center text-[--color-text-muted]">
              {hasActiveFilters ? 'No plans match your filters.' : 'No plans found.'}
            </div>
          ) : (
            plans.map((p) => (
              <div
                key={p.id}
                className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-4 hover:border-[--color-primary] transition-[--transition-fast]"
              >
                <div className="flex items-start justify-between mb-2">
                  <h3 className="font-semibold text-[--color-text-primary]">{p.name}</h3>
                  <StatusBadge status={p.status} variant="compact" />
                </div>
                <p className="text-sm text-[--color-text-secondary]">
                  {formatPricingModel(p.pricing_model)}
                </p>
                <p className="text-xs text-[--color-text-muted] mt-1">
                  {p.included_seats} {p.included_seats === 1 ? 'seat' : 'seats'} included
                </p>
                {p.metered_dimensions.length > 0 && (
                  <p className="text-xs text-[--color-text-muted] mt-1">
                    {formatDimensions(p.metered_dimensions)}
                  </p>
                )}
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
