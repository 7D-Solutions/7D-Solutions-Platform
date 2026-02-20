// ============================================================
// FeaturesTab — Effective entitlements with source attribution
// Shows server-derived entitlements with plan/bundle/override source,
// search filtering, and empty/loading/error states.
// ============================================================
'use client';

import { useState, useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useSearchStore } from '@/infrastructure/state/useSearchStore';
import { useSearchDebounce } from '@/infrastructure/hooks/useSearchDebounce';
import { REFETCH_INTERVAL_MS } from '@/lib/constants';
import type { EffectiveEntitlementListResponse } from '@/lib/api/types';

// ── Data fetcher ─────────────────────────────────────────────

async function fetchEffectiveFeatures(tenantId: string): Promise<EffectiveEntitlementListResponse> {
  const res = await fetch(`/api/tenants/${encodeURIComponent(tenantId)}/features/effective`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

// ── Source badge ─────────────────────────────────────────────

const SOURCE_STYLES: Record<string, { label: string; classes: string }> = {
  plan:     { label: 'Plan',     classes: 'bg-green-100 text-green-800' },
  bundle:   { label: 'Bundle',   classes: 'bg-blue-100 text-blue-800' },
  override: { label: 'Override', classes: 'bg-yellow-100 text-yellow-800' },
};

function SourceBadge({ source }: { source: string }) {
  const style = SOURCE_STYLES[source] ?? { label: source, classes: 'bg-gray-100 text-gray-700' };
  return (
    <span
      className={`px-2.5 py-0.5 text-xs font-medium rounded-full ${style.classes}`}
      data-testid="source-badge"
    >
      {style.label}
    </span>
  );
}

// ── Granted value display ────────────────────────────────────

function formatGranted(value: string | number | boolean): string {
  if (typeof value === 'boolean') return value ? 'Yes' : 'No';
  return String(value);
}

// ── Component ────────────────────────────────────────────────

interface FeaturesTabProps {
  tenantId: string;
}

export function FeaturesTab({ tenantId }: FeaturesTabProps) {
  const [searchInput, setSearchInput] = useState('');
  const [debouncedSearch, setDebouncedSearch] = useState('');
  const [sourceFilter, setSourceFilter] = useState('');
  const [debounceTimer, setDebounceTimer] = useState<ReturnType<typeof setTimeout> | null>(null);

  const featuresQuery = useQuery({
    queryKey: ['tenant', tenantId, 'features', 'effective'],
    queryFn: () => fetchEffectiveFeatures(tenantId),
    refetchInterval: REFETCH_INTERVAL_MS,
  });

  const entitlements = featuresQuery.data?.entitlements ?? [];

  // Client-side search + source filter
  const filtered = useMemo(() => {
    let result = entitlements;
    if (debouncedSearch) {
      const q = debouncedSearch.toLowerCase();
      result = result.filter(
        (e) =>
          e.code.toLowerCase().includes(q) ||
          e.name.toLowerCase().includes(q) ||
          (e.source_name?.toLowerCase().includes(q) ?? false),
      );
    }
    if (sourceFilter) {
      result = result.filter((e) => e.source === sourceFilter);
    }
    return result;
  }, [entitlements, debouncedSearch, sourceFilter]);

  function handleSearchChange(value: string) {
    setSearchInput(value);
    if (debounceTimer) clearTimeout(debounceTimer);
    const timer = setTimeout(() => setDebouncedSearch(value), SEARCH_DEBOUNCE_MS);
    setDebounceTimer(timer);
  }

  return (
    <div data-testid="features-tab">
      <div className="flex items-center justify-between mb-4">
        <h2 className="text-lg font-semibold text-[--color-text-primary]">
          Effective Entitlements
        </h2>
        <span className="text-sm text-[--color-text-muted]">
          {featuresQuery.data ? `${featuresQuery.data.total} total` : ''}
        </span>
      </div>

      {/* Search + source filter */}
      <div className="flex gap-3 mb-4" data-testid="features-filters">
        <input
          type="text"
          placeholder="Search by code, name, or source..."
          value={searchInput}
          onChange={(e) => handleSearchChange(e.target.value)}
          className="flex-1 px-3 py-2 text-sm rounded-[--radius-md] border border-[--color-border-default] bg-[--color-bg-primary] text-[--color-text-primary] placeholder:text-[--color-text-muted] focus:outline-none focus:ring-2 focus:ring-[--color-primary]"
          data-testid="features-search"
        />
        <select
          value={sourceFilter}
          onChange={(e) => setSourceFilter(e.target.value)}
          className="px-3 py-2 text-sm rounded-[--radius-md] border border-[--color-border-default] bg-[--color-bg-primary] text-[--color-text-primary] focus:outline-none focus:ring-2 focus:ring-[--color-primary]"
          data-testid="features-source-filter"
        >
          <option value="">All sources</option>
          <option value="plan">Plan</option>
          <option value="bundle">Bundle</option>
          <option value="override">Override</option>
        </select>
      </div>

      {/* Content states */}
      {featuresQuery.isLoading ? (
        <div
          className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-8 text-center text-[--color-text-muted]"
          data-testid="features-loading"
        >
          Loading features...
        </div>
      ) : featuresQuery.isError ? (
        <div
          className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-8 text-center text-[--color-danger]"
          data-testid="features-error"
        >
          Unable to load features
        </div>
      ) : filtered.length === 0 ? (
        <div
          className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-8 text-center text-[--color-text-muted]"
          data-testid="features-empty"
        >
          {entitlements.length === 0
            ? 'No entitlements configured for this tenant.'
            : 'No entitlements match your search.'}
        </div>
      ) : (
        <div
          className="rounded-[--radius-lg] border border-[--color-border-light] overflow-hidden"
          data-testid="features-table"
        >
          <table className="w-full border-collapse text-sm">
            <thead>
              <tr className="border-b border-[--color-border-light] bg-[--color-bg-secondary]">
                <th className="px-4 py-3 text-left text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">
                  Code
                </th>
                <th className="px-4 py-3 text-left text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">
                  Name
                </th>
                <th className="px-4 py-3 text-left text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">
                  Granted
                </th>
                <th className="px-4 py-3 text-left text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">
                  Source
                </th>
                <th className="px-4 py-3 text-left text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">
                  Details
                </th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((ent) => (
                <tr
                  key={ent.code}
                  className="border-b border-[--color-border-light] hover:bg-[--color-bg-secondary] transition-[--transition-fast]"
                  data-testid="feature-row"
                >
                  <td className="px-4 py-3 font-mono text-xs text-[--color-text-primary]">
                    {ent.code}
                  </td>
                  <td className="px-4 py-3 text-[--color-text-primary]">
                    {ent.name}
                  </td>
                  <td className="px-4 py-3 text-[--color-text-primary] font-medium">
                    {formatGranted(ent.granted)}
                  </td>
                  <td className="px-4 py-3">
                    <SourceBadge source={ent.source} />
                  </td>
                  <td className="px-4 py-3 text-[--color-text-secondary] text-xs">
                    {ent.source_name && (
                      <span>from {ent.source_name}</span>
                    )}
                    {ent.justification && (
                      <span className="italic" data-testid="feature-justification">
                        {ent.justification}
                      </span>
                    )}
                    {!ent.source_name && !ent.justification && '—'}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
