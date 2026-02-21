// ============================================================
// FeaturesTab — Effective entitlements with source attribution
// Shows server-derived entitlements with plan/bundle/override source,
// search filtering, Grant/Revoke override actions with justification modal.
// ============================================================
'use client';

import { useState, useMemo } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useSearchStore } from '@/infrastructure/state/useSearchStore';
import { useSearchDebounce } from '@/infrastructure/hooks/useSearchDebounce';
import { Button, Modal } from '@/components/ui';
import { REFETCH_INTERVAL_MS } from '@/lib/constants';
import { FeatureOverrideRequestSchema } from '@/lib/api/types';
import type { EffectiveEntitlement, EffectiveEntitlementListResponse } from '@/lib/api/types';

// ── Data fetcher ─────────────────────────────────────────────

async function fetchEffectiveFeatures(tenantId: string): Promise<EffectiveEntitlementListResponse> {
  const res = await fetch(`/api/tenants/${encodeURIComponent(tenantId)}/features/effective`);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

async function submitOverride(
  tenantId: string,
  payload: { entitlement_code: string; action: 'grant' | 'revoke'; justification: string },
): Promise<void> {
  const res = await fetch(
    `/api/tenants/${encodeURIComponent(tenantId)}/features/override`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    },
  );
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: 'Unknown error' }));
    throw new Error(body.error ?? `HTTP ${res.status}`);
  }
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

// ── Override modal ───────────────────────────────────────────

interface OverrideTarget {
  entitlement: EffectiveEntitlement;
  action: 'grant' | 'revoke';
}

function OverrideModal({
  target,
  tenantId,
  onClose,
  onSuccess,
}: {
  target: OverrideTarget;
  tenantId: string;
  onClose: () => void;
  onSuccess: () => void;
}) {
  const [justification, setJustification] = useState('');
  const [validationError, setValidationError] = useState('');

  const mutation = useMutation({
    mutationFn: () =>
      submitOverride(tenantId, {
        entitlement_code: target.entitlement.code,
        action: target.action,
        justification,
      }),
    onSuccess: () => {
      onSuccess();
      onClose();
    },
  });

  const handleSubmit = () => {
    // Client-side validation using the same Zod schema
    const result = FeatureOverrideRequestSchema.shape.justification.safeParse(justification);
    if (!result.success) {
      setValidationError(result.error.issues[0]?.message ?? 'Justification is required');
      return;
    }
    setValidationError('');
    mutation.mutate();
  };

  const actionLabel = target.action === 'grant' ? 'Grant Override' : 'Revoke Override';
  const actionVariant = target.action === 'grant' ? 'primary' : 'danger';

  return (
    <Modal
      isOpen
      title={actionLabel}
      onClose={onClose}
      size="sm"
    >
      <Modal.Body>
        <div className="space-y-4">
          <p className="text-sm text-[--color-text-primary]">
            {target.action === 'grant'
              ? <>Grant an override for <strong>{target.entitlement.name}</strong> (<code className="text-xs">{target.entitlement.code}</code>).</>
              : <>Revoke the override for <strong>{target.entitlement.name}</strong> (<code className="text-xs">{target.entitlement.code}</code>). The entitlement will revert to its plan/bundle default.</>
            }
          </p>

          <div>
            <label
              htmlFor="override-justification"
              className="block text-sm font-medium text-[--color-text-primary] mb-1"
            >
              Justification <span className="text-[--color-danger]">*</span>
            </label>
            <textarea
              id="override-justification"
              value={justification}
              onChange={(e) => {
                setJustification(e.target.value);
                if (validationError) setValidationError('');
              }}
              placeholder="Explain why this override is needed..."
              rows={3}
              maxLength={500}
              className="w-full px-3 py-2 text-sm rounded-[--radius-md] border border-[--color-border-default] bg-[--color-bg-primary] text-[--color-text-primary] placeholder:text-[--color-text-muted] focus:outline-none focus:ring-2 focus:ring-[--color-primary] resize-none"
              data-testid="override-justification"
            />
            <div className="flex justify-between mt-1">
              <span className="text-xs text-[--color-danger]" data-testid="override-validation-error">
                {validationError}
              </span>
              <span className="text-xs text-[--color-text-muted]">
                {justification.length}/500
              </span>
            </div>
          </div>

          {mutation.isError && (
            <p className="text-sm text-[--color-danger]" data-testid="override-mutation-error">
              {mutation.error.message}
            </p>
          )}
        </div>
      </Modal.Body>
      <Modal.Actions>
        <Button
          variant="ghost"
          size="sm"
          onClick={onClose}
          // Cancel is not a mutation — no cooldown needed
          disableCooldown
        >
          Cancel
        </Button>
        <Button
          variant={actionVariant as 'primary' | 'danger'}
          size="sm"
          loading={mutation.isPending}
          onClick={handleSubmit}
          data-testid="override-confirm-btn"
        >
          {actionLabel}
        </Button>
      </Modal.Actions>
    </Modal>
  );
}

// ── Component ────────────────────────────────────────────────

interface FeaturesTabProps {
  tenantId: string;
}

export function FeaturesTab({ tenantId }: FeaturesTabProps) {
  const queryClient = useQueryClient();
  const { searchTerm, setSearchTerm } = useSearchStore('features');
  const debouncedSearch = useSearchDebounce(searchTerm);
  const [sourceFilter, setSourceFilter] = useState('');
  const [overrideTarget, setOverrideTarget] = useState<OverrideTarget | null>(null);

  const featuresQuery = useQuery({
    queryKey: ['tenant', tenantId, 'features', 'effective'],
    queryFn: () => fetchEffectiveFeatures(tenantId),
    refetchInterval: REFETCH_INTERVAL_MS,
  });

  const entitlements = useMemo(
    () => featuresQuery.data?.entitlements ?? [],
    [featuresQuery.data?.entitlements],
  );

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

  const handleOverrideSuccess = () => {
    queryClient.invalidateQueries({ queryKey: ['tenant', tenantId, 'features', 'effective'] });
  };

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
          value={searchTerm}
          onChange={(e) => setSearchTerm(e.target.value)}
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
                <th className="px-4 py-3 text-right text-xs font-semibold text-[--color-text-secondary] uppercase tracking-wide">
                  Actions
                </th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((ent) => (
                <tr
                  key={ent.code}
                  className="border-b border-[--color-border-light] hover:bg-[--color-bg-secondary] [transition:var(--transition-fast)]"
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
                  <td className="px-4 py-3 text-right">
                    <OverrideActions
                      entitlement={ent}
                      onAction={(action) => setOverrideTarget({ entitlement: ent, action })}
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* Override modal */}
      {overrideTarget && (
        <OverrideModal
          target={overrideTarget}
          tenantId={tenantId}
          onClose={() => setOverrideTarget(null)}
          onSuccess={handleOverrideSuccess}
        />
      )}
    </div>
  );
}

// ── Override action buttons per row ──────────────────────────

function OverrideActions({
  entitlement,
  onAction,
}: {
  entitlement: EffectiveEntitlement;
  onAction: (action: 'grant' | 'revoke') => void;
}) {
  // Override source: show Revoke button
  if (entitlement.source === 'override') {
    return (
      <Button
        variant="danger"
        size="xs"
        onClick={() => onAction('revoke')}
        data-testid="override-revoke-btn"
      >
        Revoke
      </Button>
    );
  }

  // Plan or bundle source: show Grant Override button
  return (
    <Button
      variant="outline"
      size="xs"
      onClick={() => onAction('grant')}
      data-testid="override-grant-btn"
    >
      Override
    </Button>
  );
}
