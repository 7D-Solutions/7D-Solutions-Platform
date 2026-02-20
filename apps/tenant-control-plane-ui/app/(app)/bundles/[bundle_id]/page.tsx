// ============================================================
// /app/bundles/[bundle_id] — Bundle detail showing composition
// (entitlements list). Fetches via BFF /api/bundles/[id].
// ============================================================
'use client';
import { useParams } from 'next/navigation';
import { useQuery } from '@tanstack/react-query';
import { ArrowLeft } from 'lucide-react';
import Link from 'next/link';
import { StatusBadge } from '@/components/ui';
import { REFETCH_INTERVAL_MS } from '@/lib/constants';
import type { BundleDetail } from '@/lib/api/types';

// ── Data fetcher ────────────────────────────────────────────

async function fetchBundleDetail(bundleId: string): Promise<BundleDetail> {
  const res = await fetch(`/api/bundles/${encodeURIComponent(bundleId)}`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
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

function formatEntitlementValue(value: string | number | boolean): string {
  if (typeof value === 'boolean') return value ? 'Yes' : 'No';
  return String(value);
}

// ── Page component ──────────────────────────────────────────

export default function BundleDetailPage() {
  const params = useParams<{ bundle_id: string }>();
  const bundleId = params.bundle_id;

  const { data: bundle, isLoading, isError } = useQuery({
    queryKey: ['bundle-detail', bundleId],
    queryFn: () => fetchBundleDetail(bundleId),
    refetchInterval: REFETCH_INTERVAL_MS,
  });

  if (isLoading) {
    return (
      <div className="py-12 text-center text-[--color-text-muted]">
        Loading bundle...
      </div>
    );
  }

  if (isError || !bundle) {
    return (
      <div>
        <Link
          href="/bundles"
          className="inline-flex items-center gap-1 text-sm text-[--color-primary] hover:underline mb-4"
        >
          <ArrowLeft className="h-4 w-4" />
          Back to Bundles
        </Link>
        <div className="rounded-[--radius-lg] border border-[--color-danger] bg-red-50 p-4 text-sm text-[--color-danger]">
          Failed to load bundle detail. The bundle may not exist or the service is unavailable.
        </div>
      </div>
    );
  }

  return (
    <div>
      {/* Back link */}
      <Link
        href="/bundles"
        className="inline-flex items-center gap-1 text-sm text-[--color-primary] hover:underline mb-4"
        data-testid="bundle-back-link"
      >
        <ArrowLeft className="h-4 w-4" />
        Back to Bundles
      </Link>

      {/* Header */}
      <div className="flex items-start justify-between mb-6">
        <div>
          <h1
            className="text-2xl font-semibold text-[--color-text-primary] mb-1"
            data-testid="bundle-detail-name"
          >
            {bundle.name}
          </h1>
          {bundle.description && (
            <p className="text-sm text-[--color-text-secondary]" data-testid="bundle-detail-description">
              {bundle.description}
            </p>
          )}
        </div>
        <StatusBadge status={bundle.status} />
      </div>

      {/* Metadata */}
      <div className="flex gap-6 text-sm text-[--color-text-secondary] mb-6">
        <span>Created: {formatDate(bundle.created_at)}</span>
        <span>Updated: {formatDate(bundle.updated_at)}</span>
      </div>

      {/* Composition — entitlements */}
      <div>
        <h2 className="text-lg font-semibold text-[--color-text-primary] mb-3">
          Composition ({bundle.entitlements.length} {bundle.entitlements.length === 1 ? 'entitlement' : 'entitlements'})
        </h2>

        {bundle.entitlements.length === 0 ? (
          <div
            className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-secondary] p-8 text-center text-sm text-[--color-text-muted]"
            data-testid="bundle-empty-composition"
          >
            This bundle has no entitlements.
          </div>
        ) : (
          <div
            className="rounded-[--radius-lg] border border-[--color-border-light] overflow-hidden"
            data-testid="bundle-composition-table"
          >
            <table className="w-full border-collapse" style={{ fontSize: 'var(--table-body-font-size)' }}>
              <thead>
                <tr className="border-b border-[--color-border-light] bg-[--color-bg-secondary]">
                  <th className="px-4 py-2 text-left text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">Label</th>
                  <th className="px-4 py-2 text-left text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">Key</th>
                  <th className="px-4 py-2 text-left text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">Type</th>
                  <th className="px-4 py-2 text-left text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">Value</th>
                </tr>
              </thead>
              <tbody>
                {bundle.entitlements.map((ent) => (
                  <tr
                    key={ent.id}
                    className="border-b border-[--color-border-light] hover:bg-[--color-bg-secondary] transition-[--transition-fast]"
                  >
                    <td className="px-4 py-2 text-[--color-text-primary] font-medium">{ent.label}</td>
                    <td className="px-4 py-2 text-[--color-text-secondary] font-mono text-xs">{ent.key}</td>
                    <td className="px-4 py-2 text-[--color-text-secondary]">{ent.value_type}</td>
                    <td className="px-4 py-2 text-[--color-text-primary]">{formatEntitlementValue(ent.value)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}
