"use client";

import { Badge } from "@/components/ui/primitives/Badge";
import { Separator } from "@/components/ui/primitives/Separator";
import { getAnimalStats, MOCK_ANIMALS } from "@/lib/mock-animals";
import Link from "next/link";

export default function DashboardPage() {
  const { total, byStatus, bySpecies } = getAnimalStats();

  return (
    <div className="max-w-5xl space-y-6">
      <div>
        <h1 className="text-2xl font-bold text-text-primary">Dashboard</h1>
        <p className="text-sm text-text-secondary mt-1">
          Live overview of your herd — {total} animals tracked
        </p>
      </div>

      {/* Status summary cards */}
      <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <StatCard
          label="Healthy"
          value={byStatus.healthy}
          variant="success"
        />
        <StatCard
          label="Monitor"
          value={byStatus.monitor}
          variant="warning"
        />
        <StatCard
          label="Treatment"
          value={byStatus.treatment}
          variant="danger"
        />
        <StatCard
          label="Quarantine"
          value={byStatus.quarantine}
          variant="info"
        />
      </div>

      {/* Species breakdown */}
      <section>
        <h2 className="text-sm font-semibold text-text-secondary uppercase tracking-wide mb-3">
          By Species
        </h2>
        <div className="flex flex-wrap gap-2">
          {Object.entries(bySpecies).map(([species, count]) => (
            <span
              key={species}
              className="inline-flex items-center gap-1.5 rounded-full border border-border bg-bg-secondary px-3 py-1 text-sm text-text-primary"
            >
              <span className="capitalize font-medium">{species}</span>
              <span className="text-text-muted">{count}</span>
            </span>
          ))}
        </div>
      </section>

      <Separator />

      {/* Alerts — animals needing attention */}
      <section>
        <h2 className="text-base font-semibold text-text-primary mb-3">
          Needs Attention
        </h2>
        {MOCK_ANIMALS.filter(
          (a) => a.status === "monitor" || a.status === "treatment" || a.status === "quarantine"
        ).length === 0 ? (
          <p className="text-sm text-text-muted">No animals currently need attention.</p>
        ) : (
          <div className="space-y-2">
            {MOCK_ANIMALS.filter(
              (a) =>
                a.status === "monitor" ||
                a.status === "treatment" ||
                a.status === "quarantine"
            ).map((animal) => (
              <Link
                key={animal.id}
                href={`/animals/${animal.id}`}
                className="flex items-center justify-between rounded-lg border border-border bg-bg-primary px-4 py-3 hover:bg-bg-secondary transition-colors"
              >
                <div className="flex items-center gap-3 min-w-0">
                  <span className="font-medium text-text-primary text-sm truncate">
                    {animal.name ?? animal.tagNumber}
                    {animal.name && (
                      <span className="ml-1.5 text-text-muted font-normal">
                        {animal.tagNumber}
                      </span>
                    )}
                  </span>
                  <span className="text-xs text-text-muted capitalize hidden sm:block">
                    {animal.species} · {animal.pasture}
                  </span>
                </div>
                <div className="flex items-center gap-2 shrink-0 ml-3">
                  <StatusBadge status={animal.status} />
                </div>
              </Link>
            ))}
          </div>
        )}
      </section>
    </div>
  );
}

function StatCard({
  label,
  value,
  variant,
}: {
  label: string;
  value: number;
  variant: "success" | "warning" | "danger" | "info";
}) {
  return (
    <div className="rounded-lg border border-border bg-bg-primary px-5 py-4">
      <p className="text-xs font-medium text-text-secondary uppercase tracking-wide">{label}</p>
      <div className="mt-2 flex items-end gap-2">
        <span className="text-3xl font-bold text-text-primary">{value}</span>
        <Badge variant={variant} dot className="mb-1" />
      </div>
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  const map: Record<string, { variant: "success" | "warning" | "danger" | "info"; label: string }> = {
    healthy: { variant: "success", label: "Healthy" },
    monitor: { variant: "warning", label: "Monitor" },
    treatment: { variant: "danger", label: "Treatment" },
    quarantine: { variant: "info", label: "Quarantine" },
  };
  const cfg = map[status] ?? { variant: "info" as const, label: status };
  return (
    <Badge variant={cfg.variant} dot>
      {cfg.label}
    </Badge>
  );
}
