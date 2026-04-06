"use client";

import { useState, useMemo } from "react";
import { useRouter } from "next/navigation";
import { DataTable } from "@/components/ui/data-table/DataTable";
import type { ColumnDef } from "@/components/ui/data-table/DataTable";
import { Badge } from "@/components/ui/primitives/Badge";
import { SearchableSelect } from "@/components/ui/forms/SearchableSelect";
import type { Animal, AnimalStatus } from "@/lib/mock-animals";
import { MOCK_ANIMALS } from "@/lib/mock-animals";

const STATUS_OPTIONS = [
  { value: "", label: "All statuses" },
  { value: "healthy", label: "Healthy" },
  { value: "monitor", label: "Monitor" },
  { value: "treatment", label: "Treatment" },
  { value: "quarantine", label: "Quarantine" },
];

const SPECIES_OPTIONS = [
  { value: "", label: "All species" },
  { value: "cattle", label: "Cattle" },
  { value: "sheep", label: "Sheep" },
  { value: "horse", label: "Horse" },
  { value: "goat", label: "Goat" },
];

const STATUS_BADGE: Record<AnimalStatus, { variant: "success" | "warning" | "danger" | "info"; label: string }> = {
  healthy: { variant: "success", label: "Healthy" },
  monitor: { variant: "warning", label: "Monitor" },
  treatment: { variant: "danger", label: "Treatment" },
  quarantine: { variant: "info", label: "Quarantine" },
};

const COLUMNS: ColumnDef<Animal>[] = [
  {
    id: "tag",
    header: "Tag",
    cell: (row) => (
      <span className="font-mono text-xs font-medium text-text-primary">{row.tagNumber}</span>
    ),
    sortValue: (row) => row.tagNumber,
    width: "110px",
  },
  {
    id: "name",
    header: "Name",
    cell: (row) => (
      <span className="text-text-primary">{row.name ?? <span className="text-text-muted italic">—</span>}</span>
    ),
    sortValue: (row) => row.name ?? "",
  },
  {
    id: "species",
    header: "Species",
    cell: (row) => <span className="capitalize text-text-secondary">{row.species}</span>,
    sortValue: (row) => row.species,
  },
  {
    id: "breed",
    header: "Breed",
    cell: (row) => <span className="text-text-secondary">{row.breed}</span>,
    sortValue: (row) => row.breed,
  },
  {
    id: "sex",
    header: "Sex",
    cell: (row) => <span className="capitalize text-text-secondary">{row.sex}</span>,
    width: "80px",
  },
  {
    id: "weight",
    header: "Weight",
    cell: (row) => (
      <span className="text-text-primary tabular-nums">{row.weightKg} kg</span>
    ),
    sortValue: (row) => row.weightKg,
    align: "right",
    width: "100px",
  },
  {
    id: "pasture",
    header: "Pasture",
    cell: (row) => <span className="text-text-secondary">{row.pasture}</span>,
    sortValue: (row) => row.pasture,
  },
  {
    id: "status",
    header: "Status",
    cell: (row) => {
      const cfg = STATUS_BADGE[row.status];
      return (
        <Badge variant={cfg.variant} dot>
          {cfg.label}
        </Badge>
      );
    },
    sortValue: (row) => row.status,
    width: "130px",
  },
  {
    id: "lastExam",
    header: "Last Exam",
    cell: (row) => (
      <span className="text-text-secondary tabular-nums text-xs">
        {new Date(row.lastExamDate).toLocaleDateString("en-AU", {
          day: "2-digit",
          month: "short",
          year: "numeric",
        })}
      </span>
    ),
    sortValue: (row) => row.lastExamDate,
    width: "120px",
  },
];

export default function AnimalsPage() {
  const router = useRouter();
  const [search, setSearch] = useState("");
  const [statusFilter, setStatusFilter] = useState("");
  const [speciesFilter, setSpeciesFilter] = useState("");

  const filtered = useMemo(() => {
    const q = search.toLowerCase();
    return MOCK_ANIMALS.filter((a) => {
      if (statusFilter && a.status !== statusFilter) return false;
      if (speciesFilter && a.species !== speciesFilter) return false;
      if (q) {
        const haystack = [a.tagNumber, a.name ?? "", a.breed, a.pasture, a.species]
          .join(" ")
          .toLowerCase();
        if (!haystack.includes(q)) return false;
      }
      return true;
    });
  }, [search, statusFilter, speciesFilter]);

  return (
    <div className="max-w-6xl space-y-4">
      <div>
        <h1 className="text-2xl font-bold text-text-primary">Animals</h1>
        <p className="text-sm text-text-secondary mt-1">
          {filtered.length} of {MOCK_ANIMALS.length} animals
        </p>
      </div>

      <DataTable<Animal>
        tableId="ranchorbit-animals"
        columns={COLUMNS}
        data={filtered}
        getRowId={(a) => a.id}
        onRowClick={(a) => router.push(`/animals/${a.id}`)}
        searchValue={search}
        onSearchChange={setSearch}
        searchPlaceholder="Search by tag, name, breed, pasture…"
        columnManagerEnabled
        toolbar={
          <div className="flex gap-2">
            <SearchableSelect
              options={STATUS_OPTIONS}
              value={statusFilter}
              onChange={setStatusFilter}
              placeholder="Status"
              className="w-40"
            />
            <SearchableSelect
              options={SPECIES_OPTIONS}
              value={speciesFilter}
              onChange={setSpeciesFilter}
              placeholder="Species"
              className="w-36"
            />
          </div>
        }
        emptyState={
          <div className="py-8 text-center">
            <p className="text-text-secondary font-medium">No animals match your filters</p>
            <p className="text-text-muted text-sm mt-1">Try adjusting the search or filters above</p>
          </div>
        }
      />
    </div>
  );
}
