// ============================================================
// /app/entitlements — Entitlements catalog with search,
// value-type filter, status filter, column manager,
// row/card toggle, and TanStack Query.
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
import {
  ENTITLEMENT_VALUE_TYPE_OPTIONS,
  ENTITLEMENT_STATUS_OPTIONS,
} from '@/lib/api/types';
import type { EntitlementListResponse, EntitlementSummary } from '@/lib/api/types';
import type { Column } from '@/infrastructure/hooks/useColumnManager';

// ── Column definitions ──────────────────────────────────────

const DEFAULT_COLUMNS: Column[] = [
  { id: 'key',           label: 'Key',           visible: true, locked: true },
  { id: 'label',         label: 'Label',         visible: true },
  { id: 'value_type',    label: 'Value Type',    visible: true },
  { id: 'default_value', label: 'Default Value', visible: true },
  { id: 'status',        label: 'Status',        visible: true },
  { id: 'created_at',    label: 'Created',       visible: false },
];

const TABLE_COLUMNS: Array<{
  id: string;
  header: string;
  accessor: keyof EntitlementSummary | ((row: EntitlementSummary) => React.ReactNode);
}> = [
  { id: 'key',           header: 'Key',           accessor: 'key' },
  { id: 'label',         header: 'Label',         accessor: 'label' },
  { id: 'value_type',    header: 'Value Type',    accessor: (row) => formatValueType(row.value_type) },
  { id: 'default_value', header: 'Default Value', accessor: (row) => String(row.default_value) },
  { id: 'status',        header: 'Status',        accessor: (row) => <StatusBadge status={row.status} /> },
  { id: 'created_at',    header: 'Created',       accessor: (row) => formatDate(row.created_at) },
];

function formatValueType(vt: string): string {
  const labels: Record<string, string> = {
    boolean: 'Boolean',
    number: 'Number',
    string: 'String',
  };
  return labels[vt] ?? vt;
}

function formatDate(iso?: string): string {
  if (!iso) return '\u2014';
  try {
    return new Date(iso).toLocaleDateString('en-US', {
      month: 'short', day: 'numeric', year: 'numeric',
    });
  } catch {
    return iso;
  }
}

// ── Default filters ─────────────────────────────────────────

const DEFAULT_ENTITLEMENT_FILTERS: Record<string, string> & {
  value_type: string;
  status: string;
} = {
  value_type: '',
  status: '',
};

// ── Data fetcher ────────────────────────────────────────────

async function fetchEntitlements(params: {
  search: string;
  value_type: string;
  status: string;
  page: number;
  pageSize: number;
}): Promise<EntitlementListResponse> {
  const qp = new URLSearchParams();
  if (params.search) qp.set('search', params.search);
  if (params.value_type) qp.set('value_type', params.value_type);
  if (params.status) qp.set('status', params.status);
  qp.set('page', String(params.page));
  qp.set('page_size', String(params.pageSize));

  const res = await fetch(`/api/entitlements?${qp}`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

// ── Page component ──────────────────────────────────────────

export default function EntitlementsPage() {
  const { viewMode, setViewMode } = usePersistedView('entitlements');
  const columnManager = useColumnManager('entitlement-list', DEFAULT_COLUMNS);

  const { searchTerm, setSearchTerm } = useSearchStore('entitlement-list');
  const { filters, setFilter, clearFilters, hasActiveFilters } = useFilterStore(
    'entitlement-list',
    DEFAULT_ENTITLEMENT_FILTERS,
  );

  const debouncedSearch = useSearchDebounce(searchTerm);
  const pagination = usePagination({ totalCount: 0 });

  const { data, isLoading, isError } = useQuery({
    queryKey: [
      'entitlements',
      debouncedSearch,
      filters.value_type,
      filters.status,
      pagination.page,
      pagination.pageSize,
    ],
    queryFn: () =>
      fetchEntitlements({
        search: debouncedSearch,
        value_type: filters.value_type,
        status: filters.status,
        page: pagination.page,
        pageSize: pagination.pageSize,
      }),
    refetchInterval: REFETCH_INTERVAL_MS,
  });

  const entitlements: EntitlementSummary[] = data?.entitlements ?? [];
  const totalCount = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(totalCount / pagination.pageSize));
  const hasNextPage = pagination.page < totalPages;
  const hasPrevPage = pagination.page > 1;

  return (
    <div>
      {/* Header */}
      <div className="flex items-center justify-between mb-4">
        <div>
          <h1 className="text-2xl font-semibold text-[--color-text-primary] mb-1">Entitlements</h1>
          <p className="text-sm text-[--color-text-secondary]">
            {totalCount} {totalCount === 1 ? 'entitlement' : 'entitlements'}
          </p>
        </div>
        <ViewToggle value={viewMode} onChange={setViewMode} />
      </div>

      {/* Search + Filters bar */}
      <div
        className="flex flex-wrap items-end gap-3 mb-4"
        data-testid="entitlement-filter-bar"
      >
        {/* Search input */}
        <div className="relative flex-1 min-w-[200px]">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-[--color-text-muted]" />
          <input
            type="text"
            value={searchTerm}
            onChange={(e) => setSearchTerm(e.target.value)}
            placeholder="Search entitlements..."
            className="w-full rounded-[--radius-default] border border-[--color-border-default] bg-[--color-bg-primary] pl-9 pr-3 py-2 text-sm text-[--color-text-primary] placeholder:text-[--color-text-muted] focus:outline-none focus:ring-2 focus:ring-[--color-primary] focus:border-[--color-primary]"
            data-testid="entitlement-search-input"
          />
        </div>

        {/* Value type filter */}
        <select
          value={filters.value_type}
          onChange={(e) => setFilter('value_type', e.target.value)}
          className="rounded-[--radius-default] border border-[--color-border-default] bg-[--color-bg-primary] px-3 py-2 text-sm text-[--color-text-primary]"
          data-testid="entitlement-type-filter"
          aria-label="Filter by value type"
        >
          {ENTITLEMENT_VALUE_TYPE_OPTIONS.map((opt) => (
            <option key={opt.value} value={opt.value}>{opt.label}</option>
          ))}
        </select>

        {/* Status filter */}
        <select
          value={filters.status}
          onChange={(e) => setFilter('status', e.target.value)}
          className="rounded-[--radius-default] border border-[--color-border-default] bg-[--color-bg-primary] px-3 py-2 text-sm text-[--color-text-primary]"
          data-testid="entitlement-status-filter"
          aria-label="Filter by status"
        >
          {ENTITLEMENT_STATUS_OPTIONS.map((opt) => (
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
          Failed to load entitlements. The entitlement service may be unavailable.
        </div>
      )}

      {/* Row view — DataTable with column manager */}
      {viewMode === 'row' ? (
        <div data-testid="entitlement-row-view">
          <DataTable
            data={entitlements as unknown as Record<string, unknown>[]}
            columns={TABLE_COLUMNS as DataTableColumn[]}
            columnManager={columnManager}
            keyField="id"
            loading={isLoading}
            emptyMessage={
              hasActiveFilters || debouncedSearch
                ? 'No entitlements match your search.'
                : 'No entitlements found.'
            }
          />
        </div>
      ) : (
        /* Card view */
        <div
          className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4"
          data-testid="entitlement-card-view"
        >
          {isLoading ? (
            <div className="col-span-full py-12 text-center text-[--color-text-muted]">
              Loading...
            </div>
          ) : entitlements.length === 0 ? (
            <div className="col-span-full py-12 text-center text-[--color-text-muted]">
              {hasActiveFilters || debouncedSearch
                ? 'No entitlements match your search.'
                : 'No entitlements found.'}
            </div>
          ) : (
            entitlements.map((e) => (
              <div
                key={e.id}
                className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-4"
                data-testid={`entitlement-card-${e.id}`}
              >
                <div className="flex items-start justify-between mb-2">
                  <h3 className="font-semibold text-[--color-text-primary]">{e.label}</h3>
                  <StatusBadge status={e.status} variant="compact" />
                </div>
                <p className="text-sm text-[--color-text-secondary] font-mono">{e.key}</p>
                <p className="text-xs text-[--color-text-muted] mt-1">
                  {formatValueType(e.value_type)} &middot; default: {String(e.default_value)}
                </p>
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
