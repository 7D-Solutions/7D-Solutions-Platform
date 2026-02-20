// ============================================================
// /app/bundles — Bundles catalog list with column manager,
// row/card toggle, status filter, and TanStack Query.
// Clicking a row opens bundle detail as a tab.
// ============================================================
'use client';
import { useQuery } from '@tanstack/react-query';
import { useRouter } from 'next/navigation';
import { Button, ViewToggle, DataTable, StatusBadge, Pagination } from '@/components/ui';
import { usePersistedView } from '@/infrastructure/hooks/usePersistedView';
import { useColumnManager } from '@/infrastructure/hooks/useColumnManager';
import { usePagination } from '@/infrastructure/hooks/usePagination';
import { useFilterStore } from '@/infrastructure/state/useFilterStore';
import { useTabActions } from '@/infrastructure/state/tabStore';
import { REFETCH_INTERVAL_MS } from '@/lib/constants';
import { BUNDLE_STATUS_OPTIONS } from '@/lib/api/types';
import type { BundleListResponse, BundleSummary } from '@/lib/api/types';
import type { Column } from '@/infrastructure/hooks/useColumnManager';

// ── Column definitions ──────────────────────────────────────

const DEFAULT_COLUMNS: Column[] = [
  { id: 'name',              label: 'Name',              visible: true, locked: true },
  { id: 'status',            label: 'Status',            visible: true },
  { id: 'entitlement_count', label: 'Entitlements',      visible: true },
  { id: 'created_at',        label: 'Created',           visible: false },
];

const TABLE_COLUMNS: Array<{
  id: string;
  header: string;
  accessor: keyof BundleSummary | ((row: BundleSummary) => React.ReactNode);
}> = [
  { id: 'name',              header: 'Name',         accessor: 'name' },
  { id: 'status',            header: 'Status',       accessor: (row) => <StatusBadge status={row.status} /> },
  { id: 'entitlement_count', header: 'Entitlements', accessor: (row) => String(row.entitlement_count) },
  { id: 'created_at',        header: 'Created',      accessor: (row) => formatDate(row.created_at) },
];

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

const DEFAULT_BUNDLE_FILTERS: Record<string, string> & { status: string } = {
  status: '',
};

// ── Data fetcher ────────────────────────────────────────────

async function fetchBundles(params: {
  status: string;
  page: number;
  pageSize: number;
}): Promise<BundleListResponse> {
  const qp = new URLSearchParams();
  if (params.status) qp.set('status', params.status);
  qp.set('page', String(params.page));
  qp.set('page_size', String(params.pageSize));

  const res = await fetch(`/api/bundles?${qp}`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

// ── Page component ──────────────────────────────────────────

export default function BundlesPage() {
  const router = useRouter();
  const { viewMode, setViewMode } = usePersistedView('bundles');
  const columnManager = useColumnManager('bundle-list', DEFAULT_COLUMNS);
  const { openTab } = useTabActions();

  const { filters, setFilter, clearFilters, hasActiveFilters } = useFilterStore(
    'bundle-list',
    DEFAULT_BUNDLE_FILTERS,
  );

  const pagination = usePagination({ totalCount: 0 });

  const { data, isLoading, isError } = useQuery({
    queryKey: ['bundles', filters.status, pagination.page, pagination.pageSize],
    queryFn: () =>
      fetchBundles({
        status: filters.status,
        page: pagination.page,
        pageSize: pagination.pageSize,
      }),
    refetchInterval: REFETCH_INTERVAL_MS,
  });

  const bundles: BundleSummary[] = data?.bundles ?? [];
  const totalCount = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(totalCount / pagination.pageSize));
  const hasNextPage = pagination.page < totalPages;
  const hasPrevPage = pagination.page > 1;

  const handleBundleClick = (bundle: BundleSummary) => {
    const route = `/bundles/${bundle.id}`;
    openTab({ title: bundle.name, route });
    router.push(route);
  };

  return (
    <div>
      {/* Header */}
      <div className="flex items-center justify-between mb-4">
        <div>
          <h1 className="text-2xl font-semibold text-[--color-text-primary] mb-1">Bundles &amp; Features</h1>
          <p className="text-sm text-[--color-text-secondary]">
            {totalCount} {totalCount === 1 ? 'bundle' : 'bundles'}
          </p>
        </div>
        <ViewToggle value={viewMode} onChange={setViewMode} />
      </div>

      {/* Status filter bar */}
      <div
        className="flex flex-wrap items-end gap-3 mb-4"
        data-testid="bundle-filter-bar"
      >
        <select
          value={filters.status}
          onChange={(e) => setFilter('status', e.target.value)}
          className="rounded-[--radius-default] border border-[--color-border-default] bg-[--color-bg-primary] px-3 py-2 text-sm text-[--color-text-primary]"
          data-testid="bundle-status-filter"
          aria-label="Filter by status"
        >
          {BUNDLE_STATUS_OPTIONS.map((opt) => (
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
          Failed to load bundles. The bundle service may be unavailable.
        </div>
      )}

      {/* Row view — DataTable with column manager */}
      {viewMode === 'row' ? (
        <div data-testid="bundle-row-view">
          <DataTable
            data={bundles as unknown as Record<string, unknown>[]}
            columns={TABLE_COLUMNS as DataTableColumn[]}
            columnManager={columnManager}
            keyField="id"
            loading={isLoading}
            onRowClick={(row) => handleBundleClick(row as unknown as BundleSummary)}
            emptyMessage={
              hasActiveFilters
                ? 'No bundles match your filters.'
                : 'No bundles found.'
            }
          />
        </div>
      ) : (
        /* Card view */
        <div
          className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4"
          data-testid="bundle-card-view"
        >
          {isLoading ? (
            <div className="col-span-full py-12 text-center text-[--color-text-muted]">
              Loading...
            </div>
          ) : bundles.length === 0 ? (
            <div className="col-span-full py-12 text-center text-[--color-text-muted]">
              {hasActiveFilters ? 'No bundles match your filters.' : 'No bundles found.'}
            </div>
          ) : (
            bundles.map((b) => (
              <div
                key={b.id}
                role="button"
                tabIndex={0}
                onClick={() => handleBundleClick(b)}
                onKeyDown={(e) => { if (e.key === 'Enter') handleBundleClick(b); }}
                className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-4 hover:border-[--color-primary] transition-[--transition-fast] cursor-pointer"
                data-testid={`bundle-card-${b.id}`}
              >
                <div className="flex items-start justify-between mb-2">
                  <h3 className="font-semibold text-[--color-text-primary]">{b.name}</h3>
                  <StatusBadge status={b.status} variant="compact" />
                </div>
                <p className="text-sm text-[--color-text-secondary]">
                  {b.entitlement_count} {b.entitlement_count === 1 ? 'entitlement' : 'entitlements'}
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
