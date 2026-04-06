import React from "react";
import { cn } from "../../lib/cn.js";
import { ColumnManager } from "./ColumnManager.js";
import type { ColumnManagerResult } from "../../hooks/useColumnManager.js";

export interface DataTableToolbarProps {
  /** Controlled search value */
  searchValue?: string;
  onSearchChange?: (value: string) => void;
  searchPlaceholder?: string;
  /** Pass columnManager to show the column manager button */
  columnManager?: ColumnManagerResult;
  /** Extra controls rendered after the search input (filters, action buttons, etc.) */
  children?: React.ReactNode;
  className?: string;
}

const SearchIcon = () => (
  <svg
    aria-hidden="true"
    width="14"
    height="14"
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
  >
    <circle cx="7" cy="7" r="5" />
    <line x1="11" y1="11" x2="15" y2="15" />
  </svg>
);

const ClearIcon = () => (
  <svg
    aria-hidden="true"
    width="12"
    height="12"
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
  >
    <line x1="3" y1="3" x2="13" y2="13" />
    <line x1="13" y1="3" x2="3" y2="13" />
  </svg>
);

/**
 * Toolbar row for a DataTable — search input, column manager, and optional extra controls.
 *
 * @example
 *   <DataTableToolbar
 *     searchValue={search}
 *     onSearchChange={setSearch}
 *     columnManager={colManager}
 *   >
 *     <Button size="sm" variant="outline" onClick={handleExport}>Export</Button>
 *   </DataTableToolbar>
 */
export function DataTableToolbar({
  searchValue = "",
  onSearchChange,
  searchPlaceholder = "Search…",
  columnManager,
  children,
  className,
}: DataTableToolbarProps) {
  return (
    <div
      className={cn(
        "flex items-center gap-2 flex-wrap",
        className
      )}
    >
      {/* Search input */}
      {onSearchChange !== undefined && (
        <div className="relative flex items-center">
          <span className="absolute left-2.5 text-text-muted pointer-events-none">
            <SearchIcon />
          </span>
          <input
            type="search"
            aria-label="Search table"
            value={searchValue}
            onChange={(e) => onSearchChange(e.target.value)}
            placeholder={searchPlaceholder}
            className={cn(
              "h-8 pl-8 pr-8 text-sm rounded-md border border-border bg-bg-primary text-text-primary",
              "placeholder:text-text-muted",
              "focus:outline-none focus:ring-2 focus:ring-primary focus:border-primary",
              "transition-colors duration-150",
              "w-48 focus:w-64"
            )}
          />
          {searchValue && (
            <button
              type="button"
              aria-label="Clear search"
              onClick={() => onSearchChange("")}
              className={cn(
                "absolute right-2 text-text-muted hover:text-text-primary",
                "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary rounded"
              )}
            >
              <ClearIcon />
            </button>
          )}
        </div>
      )}

      {/* Extra controls */}
      {children}

      {/* Column manager — pushed to the right */}
      {columnManager && (
        <div className="ml-auto">
          <ColumnManager columnManager={columnManager} />
        </div>
      )}
    </div>
  );
}
