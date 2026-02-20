// ============================================================
// /app/audit — Audit log with filters, pagination, detail modal
// Filters: actor, action type, tenant ID, date range
// ============================================================
'use client';
import { useCallback, useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Search, X, Eye } from 'lucide-react';
import {
  Button, DataTable, Pagination, Modal, DateRangePicker, StatusBadge,
} from '@/components/ui';
import type { DateRange } from '@/components/ui';
import { useColumnManager } from '@/infrastructure/hooks/useColumnManager';
import { usePagination } from '@/infrastructure/hooks/usePagination';
import { useSearchDebounce } from '@/infrastructure/hooks/useSearchDebounce';
import { useSearchStore } from '@/infrastructure/state/useSearchStore';
import { useFilterStore } from '@/infrastructure/state/useFilterStore';
import { useTabModal } from '@/infrastructure/state/useTabModal';
import { useModalStore } from '@/infrastructure/state/modalStore';
import { REFETCH_INTERVAL_MS } from '@/lib/constants';
import type { AuditListResponse, AuditEventSummary } from '@/lib/api/types';
import {
  DEFAULT_AUDIT_FILTERS,
  AUDIT_ACTION_OPTIONS,
} from '@/lib/api/types';
import type { Column } from '@/infrastructure/hooks/useColumnManager';

// ── Column definitions ──────────────────────────────────────

const DEFAULT_COLUMNS: Column[] = [
  { id: 'timestamp', label: 'Time',        visible: true, locked: true },
  { id: 'actor',     label: 'Actor',       visible: true },
  { id: 'action',    label: 'Action',      visible: true },
  { id: 'tenant',    label: 'Tenant',      visible: true },
  { id: 'summary',   label: 'Summary',     visible: true },
  { id: 'detail',    label: '',             visible: true },
];

function formatTimestamp(iso?: string): string {
  if (!iso) return '—';
  try {
    return new Date(iso).toLocaleString('en-US', {
      month: 'short', day: 'numeric', year: 'numeric',
      hour: 'numeric', minute: '2-digit',
    });
  } catch {
    return iso;
  }
}

/** Safely truncate a string for table display */
function truncate(str: string | undefined, max: number): string {
  if (!str) return '—';
  return str.length > max ? `${str.slice(0, max)}...` : str;
}

/** Safely format payload as indented JSON for the detail modal */
function safeJsonDisplay(payload: unknown): string {
  if (payload === undefined || payload === null) return 'No payload';
  try {
    return JSON.stringify(payload, null, 2);
  } catch {
    return String(payload);
  }
}

// ── Data fetcher ────────────────────────────────────────────

async function fetchAuditEvents(params: {
  actor: string;
  action: string;
  tenant_id: string;
  date_from: string;
  date_to: string;
  page: number;
  pageSize: number;
}): Promise<AuditListResponse> {
  const qp = new URLSearchParams();
  if (params.actor) qp.set('actor', params.actor);
  if (params.action) qp.set('action', params.action);
  if (params.tenant_id) qp.set('tenant_id', params.tenant_id);
  if (params.date_from) qp.set('date_from', params.date_from);
  if (params.date_to) qp.set('date_to', params.date_to);
  qp.set('page', String(params.page));
  qp.set('page_size', String(params.pageSize));

  const res = await fetch(`/api/audit?${qp}`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

// ── Page component ──────────────────────────────────────────

export default function AuditPage() {
  const columnManager = useColumnManager('audit-log', DEFAULT_COLUMNS);

  // Zustand stores
  const { searchTerm, setSearchTerm } = useSearchStore('audit-log');
  const { filters, setFilter, clearFilters, hasActiveFilters } = useFilterStore(
    'audit-log',
    DEFAULT_AUDIT_FILTERS,
  );

  const debouncedActor = useSearchDebounce(searchTerm);

  // Date range derived from filter store (date_from / date_to fields)
  const dateRange: DateRange = useMemo(
    () => ({ start: String(filters.date_from ?? ''), end: String(filters.date_to ?? '') }),
    [filters.date_from, filters.date_to],
  );
  const setDateRange = useCallback(
    (range: DateRange) => {
      setFilter('date_from', range.start);
      setFilter('date_to', range.end);
    },
    [setFilter],
  );

  // Detail modal via tab-aware modal store (ESLint: no local selection state)
  const AUDIT_DETAIL_MODAL = 'AUDIT_DETAIL';
  const { openModal, closeModal } = useTabModal();
  const detailModal = useModalStore((s) => s.getModal(AUDIT_DETAIL_MODAL));
  const detailEvent = (detailModal?.props?.event as AuditEventSummary) ?? null;

  const pagination = usePagination({ totalCount: 0 });

  const { data, isLoading, isError } = useQuery({
    queryKey: [
      'audit-events',
      debouncedActor,
      filters.action,
      filters.tenant_id,
      dateRange.start,
      dateRange.end,
      pagination.page,
      pagination.pageSize,
    ],
    queryFn: () =>
      fetchAuditEvents({
        actor: debouncedActor,
        action: filters.action,
        tenant_id: filters.tenant_id,
        date_from: dateRange.start,
        date_to: dateRange.end,
        page: pagination.page,
        pageSize: pagination.pageSize,
      }),
    refetchInterval: REFETCH_INTERVAL_MS,
  });

  const events: AuditEventSummary[] = data?.events ?? [];
  const totalCount = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(totalCount / pagination.pageSize));
  const hasNextPage = pagination.page < totalPages;
  const hasPrevPage = pagination.page > 1;

  const handleClearAll = useCallback(() => {
    clearFilters();
    setSearchTerm('');
  }, [clearFilters, setSearchTerm]);

  const hasAnyFilter = hasActiveFilters || !!debouncedActor;

  // Table column definitions with render functions
  const TABLE_COLUMNS: Array<{
    id: string;
    header: string;
    accessor: keyof Record<string, unknown> | ((row: Record<string, unknown>) => React.ReactNode);
  }> = [
    { id: 'timestamp', header: 'Time',    accessor: (row) => formatTimestamp((row as unknown as AuditEventSummary).timestamp) },
    { id: 'actor',     header: 'Actor',   accessor: (row) => truncate((row as unknown as AuditEventSummary).actor, 40) },
    { id: 'action',    header: 'Action',  accessor: (row) => {
      const e = row as unknown as AuditEventSummary;
      return <StatusBadge status={e.action} />;
    }},
    { id: 'tenant',    header: 'Tenant',  accessor: (row) => {
      const e = row as unknown as AuditEventSummary;
      return e.tenant_name ? truncate(e.tenant_name, 30) : (e.tenant_id ? truncate(e.tenant_id, 20) : '—');
    }},
    { id: 'summary',   header: 'Summary', accessor: (row) => truncate((row as unknown as AuditEventSummary).summary, 60) },
    { id: 'detail',    header: '',        accessor: (row) => (
      <Button
        variant="ghost"
        size="sm"
        onClick={(evt: React.MouseEvent) => {
          evt.stopPropagation();
          openModal(AUDIT_DETAIL_MODAL, 'AUDIT_DETAIL', { event: row as unknown as AuditEventSummary });
        }}
        aria-label="View event detail"
        data-testid="audit-detail-btn"
        icon={Eye}
        iconPosition="left"
      />
    )},
  ];

  return (
    <div>
      {/* Header */}
      <div className="flex items-center justify-between mb-4">
        <div>
          <h1 className="text-2xl font-semibold text-[--color-text-primary] mb-1">Audit Log</h1>
          <p className="text-sm text-[--color-text-secondary]">
            {totalCount} {totalCount === 1 ? 'event' : 'events'}
          </p>
        </div>
      </div>

      {/* Filters bar */}
      <div className="flex flex-wrap items-end gap-3 mb-4" data-testid="audit-filter-bar">
        {/* Actor search */}
        <div className="relative flex-1 min-w-[180px]">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-[--color-text-muted]" />
          <input
            type="text"
            value={searchTerm}
            onChange={(e) => setSearchTerm(e.target.value)}
            placeholder="Search by actor..."
            className="w-full rounded-[--radius-default] border border-[--color-border-default] bg-[--color-bg-primary] pl-9 pr-3 py-2 text-sm text-[--color-text-primary] placeholder:text-[--color-text-muted] focus:outline-none focus:ring-2 focus:ring-[--color-primary] focus:border-[--color-primary]"
            data-testid="audit-actor-search"
          />
        </div>

        {/* Action type filter */}
        <select
          value={filters.action}
          onChange={(e) => setFilter('action', e.target.value)}
          className="rounded-[--radius-default] border border-[--color-border-default] bg-[--color-bg-primary] px-3 py-2 text-sm text-[--color-text-primary]"
          data-testid="audit-action-filter"
          aria-label="Filter by action type"
        >
          {AUDIT_ACTION_OPTIONS.map((opt) => (
            <option key={opt.value} value={opt.value}>{opt.label}</option>
          ))}
        </select>

        {/* Tenant ID filter */}
        <input
          type="text"
          value={filters.tenant_id}
          onChange={(e) => setFilter('tenant_id', e.target.value)}
          placeholder="Tenant ID..."
          className="w-[160px] rounded-[--radius-default] border border-[--color-border-default] bg-[--color-bg-primary] px-3 py-2 text-sm text-[--color-text-primary] placeholder:text-[--color-text-muted] focus:outline-none focus:ring-2 focus:ring-[--color-primary] focus:border-[--color-primary]"
          data-testid="audit-tenant-filter"
          aria-label="Filter by tenant ID"
        />

        {/* Date range */}
        <DateRangePicker
          value={dateRange}
          onChange={setDateRange}
        />

        {/* Clear all filters */}
        {hasAnyFilter && (
          <Button
            variant="ghost"
            size="sm"
            onClick={handleClearAll}
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
          Failed to load audit events. The audit service may be unavailable.
        </div>
      )}

      {/* DataTable */}
      <div data-testid="audit-table">
        <DataTable
          data={events as unknown as Record<string, unknown>[]}
          columns={TABLE_COLUMNS}
          columnManager={columnManager}
          keyField="id"
          loading={isLoading}
          emptyMessage={
            hasAnyFilter
              ? 'No audit events match your filters.'
              : 'No audit events found.'
          }
        />
      </div>

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

      {/* Event Detail Modal */}
      <Modal
        isOpen={!!detailEvent}
        title="Audit Event Detail"
        onClose={() => closeModal(AUDIT_DETAIL_MODAL)}
        size="lg"
      >
        <Modal.Body>
          {detailEvent && (
            <div className="space-y-4" data-testid="audit-detail-modal">
              <DetailRow label="Event ID" value={detailEvent.id} />
              <DetailRow label="Timestamp" value={formatTimestamp(detailEvent.timestamp)} />
              <DetailRow label="Actor" value={detailEvent.actor} />
              <DetailRow label="Action" value={detailEvent.action} />
              <DetailRow label="Tenant" value={detailEvent.tenant_name ?? detailEvent.tenant_id ?? '—'} />
              <DetailRow label="Resource Type" value={detailEvent.resource_type ?? '—'} />
              <DetailRow label="Resource ID" value={detailEvent.resource_id ?? '—'} />
              <DetailRow label="Summary" value={detailEvent.summary ?? '—'} />

              {/* Payload — rendered as safe pre-formatted JSON */}
              <div>
                <span className="text-xs font-medium text-[--color-text-secondary] uppercase tracking-wide">
                  Payload
                </span>
                <pre
                  className="mt-1 rounded-[--radius-default] bg-[--color-bg-secondary] border border-[--color-border-light] p-3 text-xs text-[--color-text-primary] overflow-x-auto whitespace-pre-wrap break-all max-h-64"
                  data-testid="audit-payload"
                >
                  {safeJsonDisplay(detailEvent.payload)}
                </pre>
              </div>
            </div>
          )}
        </Modal.Body>
        <Modal.Actions>
          <Button variant="secondary" size="sm" onClick={() => closeModal(AUDIT_DETAIL_MODAL)}>
            Close
          </Button>
        </Modal.Actions>
      </Modal>
    </div>
  );
}

// ── Detail row helper ───────────────────────────────────────

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <span className="text-xs font-medium text-[--color-text-secondary] uppercase tracking-wide">
        {label}
      </span>
      <p className="mt-0.5 text-sm text-[--color-text-primary] break-all">{value}</p>
    </div>
  );
}
