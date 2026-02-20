// ============================================================
// /app/tenants — Tenant list with search, filters, pagination,
// row/card toggle, column manager, and TanStack Query
// ============================================================
'use client';
import { useQuery } from '@tanstack/react-query';
import { Search, X } from 'lucide-react';
import { Button, ViewToggle, DataTable, StatusBadge, Pagination } from '@/components/ui';
import { usePersistedView } from '@/infrastructure/hooks/usePersistedView';
import { useColumnManager } from '@/infrastructure/hooks/useColumnManager';
import { usePagination } from '@/infrastructure/hooks/usePagination';
import { useSearchDebounce } from '@/infrastructure/hooks/useSearchDebounce';
import { useSearchStore } from '@/infrastructure/state/useSearchStore';
import { useFilterStore } from '@/infrastructure/state/useFilterStore';
import { REFETCH_INTERVAL_MS } from '@/lib/constants';
import type { TenantListResponse, TenantSummary } from '@/lib/api/types';
import {
  DEFAULT_TENANT_FILTERS,
  TENANT_STATUS_OPTIONS,
  TENANT_PLAN_OPTIONS,
} from '@/lib/api/types';
import type { Column } from '@/infrastructure/hooks/useColumnManager';

// ── Column definitions ──────────────────────────────────────

const DEFAULT_COLUMNS: Column[] = [
  { id: 'name',       label: 'Name',       visible: true, locked: true },
  { id: 'status',     label: 'Status',     visible: true },
  { id: 'plan',       label: 'Plan',       visible: true },
  { id: 'app_id',     label: 'App ID',     visible: true },
  { id: 'created_at', label: 'Created',    visible: false },
  { id: 'updated_at', label: 'Updated',    visible: false },
];

const TABLE_COLUMNS: Array<{
  id: string;
  header: string;
  accessor: keyof TenantSummary | ((row: TenantSummary) => React.ReactNode);
}> = [
  { id: 'name',       header: 'Name',    accessor: 'name' },
  { id: 'status',     header: 'Status',  accessor: (row) => <StatusBadge status={row.status} /> },
  { id: 'plan',       header: 'Plan',    accessor: 'plan' },
  { id: 'app_id',     header: 'App ID',  accessor: (row) => row.app_id ?? '—' },
  { id: 'created_at', header: 'Created', accessor: (row) => formatDate(row.created_at) },
  { id: 'updated_at', header: 'Updated', accessor: (row) => formatDate(row.updated_at) },
];

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

// ── Data fetcher ────────────────────────────────────────────

async function fetchTenants(params: {
  search: string;
  status: string;
  plan: string;
  app_id: string;
  page: number;
  pageSize: number;
}): Promise<TenantListResponse> {
  const qp = new URLSearchParams();
  if (params.search) qp.set('search', params.search);
  if (params.status) qp.set('status', params.status);
  if (params.plan) qp.set('plan', params.plan);
  if (params.app_id) qp.set('app_id', params.app_id);
  qp.set('page', String(params.page));
  qp.set('page_size', String(params.pageSize));

  const res = await fetch(`/api/tenants?${qp}`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

// ── Page component ──────────────────────────────────────────

export default function TenantsPage() {
  const { viewMode, setViewMode } = usePersistedView('tenants');
  const columnManager = useColumnManager('tenant-list', DEFAULT_COLUMNS);

  // Zustand stores for search and filters (ESLint-enforced — no ad-hoc useState)
  const { searchTerm, setSearchTerm } = useSearchStore('tenant-list');
  const { filters, setFilter, clearFilters, hasActiveFilters } = useFilterStore(
    'tenant-list',
    DEFAULT_TENANT_FILTERS,
  );

  const debouncedSearch = useSearchDebounce(searchTerm);

  // Pagination state — page/pageSize are internal state; derived values computed from query data below
  const pagination = usePagination({ totalCount: 0 });

  // TanStack Query — key includes all filter + pagination params
  const { data, isLoading, isError } = useQuery({
    queryKey: [
      'tenants',
      debouncedSearch,
      filters.status,
      filters.plan,
      filters.app_id,
      pagination.page,
      pagination.pageSize,
    ],
    queryFn: () =>
      fetchTenants({
        search: debouncedSearch,
        status: filters.status,
        plan: filters.plan,
        app_id: filters.app_id,
        page: pagination.page,
        pageSize: pagination.pageSize,
      }),
    refetchInterval: REFETCH_INTERVAL_MS,
  });

  const tenants: TenantSummary[] = data?.tenants ?? [];
  const totalCount = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(totalCount / pagination.pageSize));
  const hasNextPage = pagination.page < totalPages;
  const hasPrevPage = pagination.page > 1;

  return (
    <div>
      {/* Header */}
      <div className="flex items-center justify-between mb-4">
        <div>
          <h1 className="text-2xl font-semibold text-[--color-text-primary] mb-1">Tenants</h1>
          <p className="text-sm text-[--color-text-secondary]">
            {totalCount} {totalCount === 1 ? 'tenant' : 'tenants'}
          </p>
        </div>
        <ViewToggle value={viewMode} onChange={setViewMode} />
      </div>

      {/* Search + Filters bar */}
      <div
        className="flex flex-wrap items-end gap-3 mb-4"
        data-testid="filter-bar"
      >
        {/* Search input */}
        <div className="relative flex-1 min-w-[200px]">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-[--color-text-muted]" />
          <input
            type="text"
            value={searchTerm}
            onChange={(e) => setSearchTerm(e.target.value)}
            placeholder="Search tenants..."
            className="w-full rounded-[--radius-default] border border-[--color-border-default] bg-[--color-bg-primary] pl-9 pr-3 py-2 text-sm text-[--color-text-primary] placeholder:text-[--color-text-muted] focus:outline-none focus:ring-2 focus:ring-[--color-primary] focus:border-[--color-primary]"
            data-testid="search-input"
          />
        </div>

        {/* Status filter */}
        <select
          value={filters.status}
          onChange={(e) => setFilter('status', e.target.value)}
          className="rounded-[--radius-default] border border-[--color-border-default] bg-[--color-bg-primary] px-3 py-2 text-sm text-[--color-text-primary]"
          data-testid="status-filter"
          aria-label="Filter by status"
        >
          {TENANT_STATUS_OPTIONS.map((opt) => (
            <option key={opt.value} value={opt.value}>{opt.label}</option>
          ))}
        </select>

        {/* Plan filter */}
        <select
          value={filters.plan}
          onChange={(e) => setFilter('plan', e.target.value)}
          className="rounded-[--radius-default] border border-[--color-border-default] bg-[--color-bg-primary] px-3 py-2 text-sm text-[--color-text-primary]"
          data-testid="plan-filter"
          aria-label="Filter by plan"
        >
          {TENANT_PLAN_OPTIONS.map((opt) => (
            <option key={opt.value} value={opt.value}>{opt.label}</option>
          ))}
        </select>

        {/* Clear filters */}
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
          Failed to load tenants. The tenant registry may be unavailable.
        </div>
      )}

      {/* Row view — DataTable with column manager */}
      {viewMode === 'row' ? (
        <div data-testid="row-view">
          <DataTable
            data={tenants as unknown as Record<string, unknown>[]}
            columns={TABLE_COLUMNS as DataTableColumn[]}
            columnManager={columnManager}
            keyField="id"
            loading={isLoading}
            emptyMessage={
              hasActiveFilters || debouncedSearch
                ? 'No tenants match your filters.'
                : 'No tenants found.'
            }
          />
        </div>
      ) : (
        /* Card view */
        <div
          className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4"
          data-testid="card-view"
        >
          {isLoading ? (
            <div className="col-span-full py-12 text-center text-[--color-text-muted]">
              Loading...
            </div>
          ) : tenants.length === 0 ? (
            <div className="col-span-full py-12 text-center text-[--color-text-muted]">
              {hasActiveFilters || debouncedSearch
                ? 'No tenants match your filters.'
                : 'No tenants found.'}
            </div>
          ) : (
            tenants.map((t) => (
              <div
                key={t.id}
                className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-4 hover:border-[--color-primary] transition-[--transition-fast]"
              >
                <div className="flex items-start justify-between mb-2">
                  <h3 className="font-semibold text-[--color-text-primary]">{t.name}</h3>
                  <StatusBadge status={t.status} variant="compact" />
                </div>
                <p className="text-sm text-[--color-text-secondary]">{t.plan}</p>
                {t.app_id && (
                  <p className="text-xs text-[--color-text-muted] mt-1">{t.app_id}</p>
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
