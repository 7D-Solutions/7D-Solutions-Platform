import type { Meta, StoryObj } from "@storybook/react";
import { DataTable, DataTableToolbar } from "@7d/ui";
import type { ColumnDef } from "@7d/ui";
import { useState } from "react";

interface Person {
  id: string;
  name: string;
  role: string;
  status: "active" | "inactive";
  joined: string;
}

const PEOPLE: Person[] = [
  { id: "1", name: "Alice Johnson", role: "Engineer", status: "active", joined: "2023-01-15" },
  { id: "2", name: "Bob Smith", role: "Designer", status: "active", joined: "2022-08-03" },
  { id: "3", name: "Carol White", role: "Manager", status: "inactive", joined: "2021-11-22" },
  { id: "4", name: "David Lee", role: "Engineer", status: "active", joined: "2024-03-10" },
  { id: "5", name: "Eve Martinez", role: "Analyst", status: "active", joined: "2023-07-01" },
];

const COLUMNS: ColumnDef<Person>[] = [
  {
    id: "name",
    header: "Name",
    cell: (row) => <span className="font-medium">{row.name}</span>,
    sortValue: (row) => row.name,
  },
  {
    id: "role",
    header: "Role",
    cell: (row) => row.role,
    sortValue: (row) => row.role,
  },
  {
    id: "status",
    header: "Status",
    cell: (row) => (
      <span className={`text-xs font-medium ${row.status === "active" ? "text-success" : "text-text-muted"}`}>
        {row.status}
      </span>
    ),
  },
  {
    id: "joined",
    header: "Joined",
    cell: (row) => row.joined,
    sortValue: (row) => row.joined,
    align: "right",
  },
];

const meta: Meta = {
  title: "Data Table/DataTable",
  tags: ["autodocs"],
  parameters: { layout: "padded" },
};

export default meta;
type Story = StoryObj;

export const Default: Story = {
  render: () => (
    <DataTable
      tableId="docs-basic"
      columns={COLUMNS}
      data={PEOPLE}
      getRowId={(r) => r.id}
    />
  ),
};

export const WithSearch: Story = {
  render: () => {
    const [search, setSearch] = useState("");
    const filtered = PEOPLE.filter(
      (p) =>
        p.name.toLowerCase().includes(search.toLowerCase()) ||
        p.role.toLowerCase().includes(search.toLowerCase())
    );
    return (
      <DataTable
        tableId="docs-search"
        columns={COLUMNS}
        data={filtered}
        getRowId={(r) => r.id}
        searchValue={search}
        onSearchChange={setSearch}
        searchPlaceholder="Search people…"
      />
    );
  },
};

export const WithSelection: Story = {
  render: () => {
    const [selected, setSelected] = useState<Set<string | number>>(new Set());
    return (
      <DataTable
        tableId="docs-selection"
        columns={COLUMNS}
        data={PEOPLE}
        getRowId={(r) => r.id}
        selectionEnabled
        selectedIds={selected}
        onSelectionChange={(ids) => setSelected(ids)}
      />
    );
  },
};

export const Loading: Story = {
  render: () => (
    <DataTable
      tableId="docs-loading"
      columns={COLUMNS}
      data={[]}
      getRowId={(r) => r.id}
      loading
      loadingRows={5}
    />
  ),
};

export const Empty: Story = {
  render: () => (
    <DataTable
      tableId="docs-empty"
      columns={COLUMNS}
      data={[]}
      getRowId={(r) => r.id}
      emptyState={
        <div className="py-12 text-center text-sm text-text-muted">
          No records found.
        </div>
      }
    />
  ),
};
