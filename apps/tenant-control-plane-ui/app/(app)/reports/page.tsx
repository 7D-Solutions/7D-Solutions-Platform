// ============================================================
// /app/reports — Reports dashboard (placeholder)
// ============================================================
import { BarChart2 } from 'lucide-react';

export default function ReportsPage() {
  return (
    <div data-testid="reports-page">
      <div className="mb-6">
        <h1 className="text-2xl font-bold text-[--color-text-primary]">Reports</h1>
        <p className="text-sm text-[--color-text-secondary] mt-1">
          Platform analytics and operational reports
        </p>
      </div>

      <div className="rounded-lg border border-[--color-border-light] bg-[--color-bg-primary] p-8 text-center">
        <BarChart2 className="h-12 w-12 text-[--color-text-muted] mx-auto mb-4" />
        <h2 className="text-lg font-semibold text-[--color-text-primary] mb-2">
          Reporting Center
        </h2>
        <p className="text-sm text-[--color-text-secondary] max-w-md mx-auto mb-6">
          Generate and export operational reports covering tenant growth,
          service utilization, billing summaries, and system health trends.
        </p>
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-4 max-w-lg mx-auto">
          <PlannedFeature title="Tenant Growth" description="Sign-ups, churn, and net growth over time" />
          <PlannedFeature title="Usage Metrics" description="API calls, storage, and compute per tenant" />
          <PlannedFeature title="Financial" description="Revenue, collections, and aging summaries" />
        </div>
      </div>
    </div>
  );
}

function PlannedFeature({ title, description }: { title: string; description: string }) {
  return (
    <div className="rounded-[--radius-default] border border-dashed border-[--color-border-default] p-3">
      <p className="text-sm font-medium text-[--color-text-primary]">{title}</p>
      <p className="text-xs text-[--color-text-muted] mt-0.5">{description}</p>
    </div>
  );
}
