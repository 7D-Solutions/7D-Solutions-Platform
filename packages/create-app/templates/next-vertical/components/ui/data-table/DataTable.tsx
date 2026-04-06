import React, { useMemo, useState } from "react";
import { cn } from "../lib/cn";
import { useColumnManager } from "../hooks/useColumnManager";
import type { Column } from "../hooks/useColumnManager";
import { DataTableToolbar } from "./DataTableToolbar";
import { SelectAllCheckbox, RowCheckbox } from "./RowSelection";
import { Skeleton } from "../primitives/Skeleton";

export type SortDirection = "asc" | "desc";

export interface SortState {
  columnId: string;
  direction: SortDirection;
}

export interface ColumnDef<T> {
  /** Must match the id used in the Column definition for useColumnManager */
  id: string;
  header: React.ReactNode;
  /** Render the cell content for a row */
  cell: (row: T) => React.ReactNode;
  /** Optional: enables client-side sorting by this column */
  sortValue?: (row: T) => string | number | Date;
  /** Fixed width, e.g. "120px" or "10%" */
  width?: string;
  align?: "left" | "center" | "right";
}

export interface DataTableProps<T> {
  /** Stable key for localStorage column persistence */
  tableId: string;
  columns: ColumnDef<T>[];
  data: T[];
  /** Return a stable, unique ID per row */
  getRowId: (row: T) => string | number;
  loading?: boolean;
  /** Number of skeleton rows to show while loading */
  loadingRows?: number;
  emptyState?: React.ReactNode;
  onRowClick?: (row: T) => void;
  // ── Selection ──
  selectionEnabled?: boolean;
  selectedIds?: Set<string | number>;
  onSelectionChange?: (ids: Set<string | number>, items: T[]) => void;
  // ── Search (toolbar) ──
  searchValue?: string;
  onSearchChange?: (value: string) => void;
  searchPlaceholder?: string;
  // ── Column manager ──
  columnManagerEnabled?: boolean;
  /** Extra toolbar content (filters, action buttons) */
  toolbar?: React.ReactNode;
  // ── Sorting ──
  /** Controlled sort state for server-side sorting */
  sortState?: SortState;
  onSortChange?: (sort: SortState | null) => void;
  className?: string;
}

const alignClass: Record<string, string> = {
  left: "text-left",
  center: "text-center",
  right: "text-right",
};

const SortIcon = ({ direction }: { direction?: SortDirection }) => (
  <svg
    aria-hidden="true"
    width="12"
    height="12"
    viewBox="0 0 12 12"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.5"
    strokeLinecap="round"
    strokeLinejoin="round"
    className="shrink-0"
  >
    {direction === "asc" ? (
      <polyline points="2 8 6 4 10 8" />
    ) : direction === "desc" ? (
      <polyline points="2 4 6 8 10 4" />
    ) : (
      <>
        <polyline points="2 4 6 8 10 4" className="opacity-30" />
        <polyline points="2 8 6 4 10 8" className="opacity-30" />
      </>
    )}
  </svg>
);

export function DataTable<T>({
  tableId,
  columns,
  data,
  getRowId,
  loading = false,
  loadingRows = 5,
  emptyState,
  onRowClick,
  selectionEnabled = false,
  selectedIds: controlledSelectedIds,
  onSelectionChange,
  searchValue,
  onSearchChange,
  searchPlaceholder,
  columnManagerEnabled = false,
  toolbar,
  sortState: controlledSort,
  onSortChange,
  className,
}: DataTableProps<T>) {
  // ── Column manager ──
  const defaultColumnDefs: Column[] = useMemo(
    () =>
      columns.map((c) => ({
        id: c.id,
        label: typeof c.header === "string" ? c.header : c.id,
        visible: true,
        align: c.align,
      })),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [tableId]
  );

  const columnManager = useColumnManager(tableId, defaultColumnDefs);
  const visibleColumns = columnManagerEnabled
    ? columns.filter((c) => columnManager.getColumnVisibility(c.id))
    : columns;

  // ── Internal sort (used when onSortChange not provided) ──
  const [internalSort, setInternalSort] = useState<SortState | null>(null);
  const sort = controlledSort !== undefined ? controlledSort : internalSort;

  const handleSortClick = (col: ColumnDef<T>) => {
    if (!col.sortValue) return;
    const next: SortState | null =
      sort?.columnId === col.id
        ? sort.direction === "asc"
          ? { columnId: col.id, direction: "desc" }
          : null
        : { columnId: col.id, direction: "asc" };

    if (onSortChange) {
      onSortChange(next);
    } else {
      setInternalSort(next);
    }
  };

  // ── Client-side sort (only when no server-side sort handler) ──
  const sortedData = useMemo(() => {
    if (!sort || onSortChange) return data;
    const col = columns.find((c) => c.id === sort.columnId);
    if (!col?.sortValue) return data;
    return [...data].sort((a, b) => {
      const va = col.sortValue!(a);
      const vb = col.sortValue!(b);
      let cmp = 0;
      if (va < vb) cmp = -1;
      else if (va > vb) cmp = 1;
      return sort.direction === "asc" ? cmp : -cmp;
    });
  }, [data, sort, columns, onSortChange]);

  // ── Internal selection (uncontrolled fallback) ──
  const [internalSelectedIds, setInternalSelectedIds] = useState<Set<string | number>>(
    new Set()
  );
  const selectedIds = controlledSelectedIds ?? internalSelectedIds;

  const handleToggleRow = (row: T) => {
    const id = getRowId(row);
    const next = new Set(selectedIds);
    if (next.has(id)) {
      next.delete(id);
    } else {
      next.add(id);
    }
    const selectedItems = sortedData.filter((r) => next.has(getRowId(r)));
    if (onSelectionChange) {
      onSelectionChange(next, selectedItems);
    } else {
      setInternalSelectedIds(next);
    }
  };

  const handleToggleAll = (checked: boolean) => {
    const next = checked ? new Set(sortedData.map(getRowId)) : new Set<string | number>();
    const selectedItems = checked ? [...sortedData] : [];
    if (onSelectionChange) {
      onSelectionChange(next, selectedItems);
    } else {
      setInternalSelectedIds(next);
    }
  };

  const allSelected = sortedData.length > 0 && sortedData.every((r) => selectedIds.has(getRowId(r)));
  const someSelected = !allSelected && sortedData.some((r) => selectedIds.has(getRowId(r)));

  const showToolbar =
    onSearchChange !== undefined || columnManagerEnabled || toolbar !== undefined;

  return (
    <div className={cn("flex flex-col gap-3", className)}>
      {/* Toolbar */}
      {showToolbar && (
        <DataTableToolbar
          searchValue={searchValue}
          onSearchChange={onSearchChange}
          searchPlaceholder={searchPlaceholder}
          columnManager={columnManagerEnabled ? columnManager : undefined}
        >
          {toolbar}
        </DataTableToolbar>
      )}

      {/* Table */}
      <div className="overflow-x-auto rounded-lg border border-border">
        <table className="min-w-full text-sm">
          <thead className="bg-bg-secondary border-b border-border">
            <tr>
              {selectionEnabled && (
                <th
                  scope="col"
                  className="w-10 px-3 py-3"
                  aria-label="Row selection"
                >
                  <SelectAllCheckbox
                    checked={allSelected}
                    indeterminate={someSelected}
                    onChange={handleToggleAll}
                    disabled={loading || sortedData.length === 0}
                  />
                </th>
              )}
              {visibleColumns.map((col) => {
                const isSorted = sort?.columnId === col.id;
                const sortable = Boolean(col.sortValue);

                return (
                  <th
                    key={col.id}
                    scope="col"
                    style={col.width ? { width: col.width } : undefined}
                    aria-sort={
                      isSorted
                        ? sort?.direction === "asc"
                          ? "ascending"
                          : "descending"
                        : sortable
                          ? "none"
                          : undefined
                    }
                    className={cn(
                      "px-3 py-3 font-medium text-text-secondary whitespace-nowrap",
                      alignClass[col.align ?? "left"],
                      sortable &&
                        "cursor-pointer select-none hover:text-text-primary hover:bg-gray-100",
                      isSorted && "text-text-primary"
                    )}
                    onClick={sortable ? () => handleSortClick(col) : undefined}
                  >
                    <span className="inline-flex items-center gap-1">
                      {col.header}
                      {sortable && (
                        <SortIcon direction={isSorted ? sort?.direction : undefined} />
                      )}
                    </span>
                  </th>
                );
              })}
            </tr>
          </thead>

          <tbody className="divide-y divide-border">
            {loading ? (
              // Skeleton rows
              Array.from({ length: loadingRows }).map((_, i) => (
                <tr key={i}>
                  {selectionEnabled && (
                    <td className="px-3 py-3">
                      <Skeleton className="h-4 w-4 rounded" />
                    </td>
                  )}
                  {visibleColumns.map((col) => (
                    <td key={col.id} className="px-3 py-3">
                      <Skeleton className="h-4 w-3/4" />
                    </td>
                  ))}
                </tr>
              ))
            ) : sortedData.length === 0 ? (
              <tr>
                <td
                  colSpan={visibleColumns.length + (selectionEnabled ? 1 : 0)}
                  className="px-3 py-12 text-center text-text-muted"
                >
                  {emptyState ?? "No records found"}
                </td>
              </tr>
            ) : (
              sortedData.map((row) => {
                const id = getRowId(row);
                const isSelected = selectedIds.has(id);

                return (
                  <tr
                    key={id}
                    data-selected={isSelected || undefined}
                    onClick={onRowClick ? () => onRowClick(row) : undefined}
                    className={cn(
                      "bg-bg-primary transition-colors duration-100",
                      onRowClick && "cursor-pointer hover:bg-bg-secondary",
                      isSelected && "bg-primary/5"
                    )}
                  >
                    {selectionEnabled && (
                      <td
                        className="w-10 px-3 py-3"
                        onClick={(e) => {
                          e.stopPropagation();
                          handleToggleRow(row);
                        }}
                      >
                        <RowCheckbox
                          checked={isSelected}
                          onChange={() => handleToggleRow(row)}
                        />
                      </td>
                    )}
                    {visibleColumns.map((col) => (
                      <td
                        key={col.id}
                        className={cn(
                          "px-3 py-3 text-text-primary",
                          alignClass[col.align ?? "left"]
                        )}
                      >
                        {col.cell(row)}
                      </td>
                    ))}
                  </tr>
                );
              })
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
