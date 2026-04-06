"use client";

import { useRouter } from "next/navigation";
import { use } from "react";
import { Badge } from "@/components/ui/primitives/Badge";
import { Button } from "@/components/ui/primitives/Button";
import { Breadcrumbs } from "@/components/ui/navigation/Breadcrumbs";
import { Separator } from "@/components/ui/primitives/Separator";
import { getAnimalById } from "@/lib/mock-animals";
import type { AnimalStatus } from "@/lib/mock-animals";

const STATUS_BADGE: Record<AnimalStatus, { variant: "success" | "warning" | "danger" | "info"; label: string }> = {
  healthy: { variant: "success", label: "Healthy" },
  monitor: { variant: "warning", label: "Monitor" },
  treatment: { variant: "danger", label: "Treatment" },
  quarantine: { variant: "info", label: "Quarantine" },
};

function fmtDate(iso: string) {
  return new Date(iso).toLocaleDateString("en-AU", {
    day: "2-digit",
    month: "long",
    year: "numeric",
  });
}

function age(dob: string): string {
  const born = new Date(dob);
  const now = new Date();
  const years = now.getFullYear() - born.getFullYear();
  const months = now.getMonth() - born.getMonth();
  const totalMonths = years * 12 + months;
  if (totalMonths < 12) return `${totalMonths}mo`;
  const y = Math.floor(totalMonths / 12);
  const m = totalMonths % 12;
  return m === 0 ? `${y}yr` : `${y}yr ${m}mo`;
}

export default function AnimalDetailPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const router = useRouter();
  const animal = getAnimalById(id);

  if (!animal) {
    return (
      <div className="max-w-2xl space-y-4">
        <Breadcrumbs
          items={[
            { label: "Animals", href: "/animals" },
            { label: "Not found" },
          ]}
        />
        <div className="rounded-lg border border-border bg-bg-primary px-6 py-12 text-center">
          <p className="text-text-primary font-medium">Animal not found</p>
          <p className="text-text-muted text-sm mt-1">ID {id} does not exist in this herd.</p>
          <Button variant="ghost" size="sm" className="mt-4" onClick={() => router.push("/animals")}>
            Back to Animals
          </Button>
        </div>
      </div>
    );
  }

  const statusCfg = STATUS_BADGE[animal.status];
  const displayName = animal.name ?? animal.tagNumber;

  return (
    <div className="max-w-2xl space-y-5">
      {/* Breadcrumb */}
      <Breadcrumbs
        items={[
          { label: "Animals", href: "/animals" },
          { label: displayName },
        ]}
      />

      {/* Header */}
      <div className="flex items-start justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold text-text-primary">{displayName}</h1>
          <p className="text-text-muted text-sm font-mono mt-0.5">{animal.tagNumber}</p>
        </div>
        <Badge variant={statusCfg.variant} size="md" dot>
          {statusCfg.label}
        </Badge>
      </div>

      {/* Core details */}
      <div className="rounded-lg border border-border bg-bg-primary divide-y divide-border">
        <SectionHeader label="Identity" />
        <DetailRow label="Species" value={<span className="capitalize">{animal.species}</span>} />
        <DetailRow label="Breed" value={animal.breed} />
        <DetailRow label="Sex" value={<span className="capitalize">{animal.sex}</span>} />
        <DetailRow label="Date of Birth" value={`${fmtDate(animal.dob)} (${age(animal.dob)})`} />

        <SectionHeader label="Current Status" />
        <DetailRow label="Pasture / Location" value={animal.pasture} />
        <DetailRow label="Weight" value={`${animal.weightKg} kg`} />
        <DetailRow label="Last Exam" value={fmtDate(animal.lastExamDate)} />

        {animal.notes && (
          <>
            <SectionHeader label="Notes" />
            <div className="px-4 py-3">
              <p className="text-sm text-text-secondary whitespace-pre-line">{animal.notes}</p>
            </div>
          </>
        )}
      </div>

      <Separator />

      {/* Actions */}
      <div className="flex gap-2">
        <Button variant="primary" size="sm" disabled title="Coming soon — requires backend">
          Record Exam
        </Button>
        <Button variant="outline" size="sm" disabled title="Coming soon — requires backend">
          Edit Details
        </Button>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => router.push("/animals")}
        >
          Back to list
        </Button>
      </div>
    </div>
  );
}

function SectionHeader({ label }: { label: string }) {
  return (
    <div className="px-4 py-2 bg-bg-secondary">
      <p className="text-xs font-semibold text-text-muted uppercase tracking-wide">{label}</p>
    </div>
  );
}

function DetailRow({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between px-4 py-3 gap-4">
      <span className="text-sm text-text-secondary shrink-0 w-36">{label}</span>
      <span className="text-sm text-text-primary text-right">{value}</span>
    </div>
  );
}
