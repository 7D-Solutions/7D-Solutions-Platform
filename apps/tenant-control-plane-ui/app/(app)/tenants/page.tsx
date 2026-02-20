// ============================================================
// /app/tenants — Tenant list (placeholder with row/card toggle)
// ============================================================
'use client';
import { ViewToggle } from '@/components/ui';
import { usePersistedView } from '@/infrastructure/hooks/usePersistedView';

const PLACEHOLDER_TENANTS = [
  { id: 't-001', name: 'Acme Corp', status: 'active', plan: 'Professional' },
  { id: 't-002', name: 'Globex Inc', status: 'suspended', plan: 'Starter' },
  { id: 't-003', name: 'Initech', status: 'active', plan: 'Enterprise' },
];

export default function TenantsPage() {
  const { viewMode, setViewMode } = usePersistedView('tenants');

  return (
    <div>
      <div className="flex items-center justify-between mb-4">
        <div>
          <h1 className="text-2xl font-semibold text-[--color-text-primary] mb-1">Tenants</h1>
          <p className="text-sm text-[--color-text-secondary]">
            {PLACEHOLDER_TENANTS.length} tenants
          </p>
        </div>
        <ViewToggle value={viewMode} onChange={setViewMode} />
      </div>

      {viewMode === 'row' ? (
        <div
          className="rounded-[--radius-lg] border border-[--color-border-light] overflow-hidden"
          data-testid="row-view"
        >
          <table className="w-full border-collapse">
            <thead>
              <tr className="border-b border-[--color-border-light] bg-[--color-bg-secondary]">
                <th className="px-4 py-2 text-left text-xs font-semibold text-[--color-text-secondary] uppercase">Name</th>
                <th className="px-4 py-2 text-left text-xs font-semibold text-[--color-text-secondary] uppercase">Status</th>
                <th className="px-4 py-2 text-left text-xs font-semibold text-[--color-text-secondary] uppercase">Plan</th>
              </tr>
            </thead>
            <tbody>
              {PLACEHOLDER_TENANTS.map((t) => (
                <tr key={t.id} className="border-b border-[--color-border-light]">
                  <td className="px-4 py-3 text-sm">{t.name}</td>
                  <td className="px-4 py-3 text-sm">{t.status}</td>
                  <td className="px-4 py-3 text-sm">{t.plan}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <div
          className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4"
          data-testid="card-view"
        >
          {PLACEHOLDER_TENANTS.map((t) => (
            <div
              key={t.id}
              className="rounded-[--radius-lg] border border-[--color-border-light] bg-[--color-bg-primary] p-4"
            >
              <h3 className="font-semibold text-[--color-text-primary]">{t.name}</h3>
              <p className="text-sm text-[--color-text-secondary] mt-1">{t.status}</p>
              <p className="text-sm text-[--color-text-muted] mt-0.5">{t.plan}</p>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
