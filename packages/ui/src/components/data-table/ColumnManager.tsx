import React, { useEffect, useRef, useState } from "react";
import { cn } from "../../lib/cn.js";
import { Keys } from "../../lib/keyboard.js";
import type { Column, ColumnManagerResult } from "../../hooks/useColumnManager.js";

export interface ColumnManagerProps {
  columnManager: ColumnManagerResult;
  /** Button trigger label */
  triggerLabel?: string;
  /** Whether the panel opens above (useful near bottom of page) */
  dropUp?: boolean;
}

const ColumnsIcon = () => (
  <svg
    aria-hidden="true"
    width="16"
    height="16"
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.5"
    strokeLinecap="round"
    strokeLinejoin="round"
  >
    <rect x="1" y="2" width="4" height="12" rx="1" />
    <rect x="6" y="2" width="4" height="12" rx="1" />
    <rect x="11" y="2" width="4" height="12" rx="1" />
  </svg>
);

const DragIcon = () => (
  <svg
    aria-hidden="true"
    width="12"
    height="12"
    viewBox="0 0 12 12"
    fill="currentColor"
  >
    <circle cx="4" cy="3" r="1" />
    <circle cx="8" cy="3" r="1" />
    <circle cx="4" cy="6" r="1" />
    <circle cx="8" cy="6" r="1" />
    <circle cx="4" cy="9" r="1" />
    <circle cx="8" cy="9" r="1" />
  </svg>
);

export function ColumnManager({
  columnManager,
  triggerLabel = "Columns",
  dropUp = false,
}: ColumnManagerProps) {
  const {
    columns,
    isEditMode,
    toggleEditMode,
    toggleVisibility,
    resetToDefault,
    getColumnVisibility,
    handleDragStart,
    handleDragOver,
    handleDrop,
    handleDragEnd,
    dragState,
  } = columnManager;

  const [open, setOpen] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    const handle = (e: MouseEvent) => {
      if (
        panelRef.current &&
        !panelRef.current.contains(e.target as Node) &&
        triggerRef.current &&
        !triggerRef.current.contains(e.target as Node)
      ) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handle);
    return () => document.removeEventListener("mousedown", handle);
  }, [open]);

  const visibleCount = columns.filter((c) => getColumnVisibility(c.id)).length;
  const lockedColumns = columns.filter((c) => c.locked);
  const manageableColumns = columns.filter((c) => !c.locked);

  return (
    <div className="relative">
      <button
        ref={triggerRef}
        type="button"
        aria-label={`${triggerLabel} — ${visibleCount} of ${columns.length} shown`}
        aria-expanded={open}
        aria-haspopup="dialog"
        onClick={() => setOpen((v) => !v)}
        onKeyDown={(e) => e.key === Keys.Escape && setOpen(false)}
        className={cn(
          "inline-flex items-center gap-1.5 px-3 h-8 text-sm rounded-md border",
          "bg-bg-primary text-text-primary border-border",
          "hover:bg-gray-100 hover:border-border-dark",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary",
          "transition-colors duration-150",
          open && "bg-gray-100"
        )}
      >
        <ColumnsIcon />
        {triggerLabel}
      </button>

      {open && (
        <div
          ref={panelRef}
          role="dialog"
          aria-label="Manage columns"
          className={cn(
            "absolute right-0 z-50 w-64",
            "bg-bg-primary border border-border rounded-lg shadow-lg",
            dropUp ? "bottom-full mb-1" : "top-full mt-1"
          )}
        >
          {/* Header */}
          <div className="flex items-center justify-between px-3 py-2 border-b border-border">
            <span className="text-sm font-medium text-text-primary">
              Columns
            </span>
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={resetToDefault}
                className={cn(
                  "text-xs text-text-muted hover:text-text-primary",
                  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary rounded"
                )}
              >
                Reset
              </button>
              <button
                type="button"
                onClick={() => {
                  if (isEditMode) toggleEditMode();
                  setOpen(false);
                }}
                aria-label="Close column manager"
                className={cn(
                  "p-0.5 rounded text-text-muted hover:text-text-primary",
                  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                )}
              >
                ✕
              </button>
            </div>
          </div>

          {/* Reorder toggle */}
          {manageableColumns.length > 1 && (
            <div className="px-3 py-2 border-b border-border">
              <button
                type="button"
                onClick={toggleEditMode}
                className={cn(
                  "inline-flex items-center gap-1.5 text-xs rounded px-2 py-1",
                  "border transition-colors duration-150",
                  isEditMode
                    ? "bg-primary text-text-inverse border-primary"
                    : "bg-bg-secondary text-text-secondary border-border hover:bg-gray-100"
                )}
              >
                {isEditMode ? "Done reordering" : "Reorder columns"}
              </button>
            </div>
          )}

          {/* Column list */}
          <ul className="py-1 max-h-72 overflow-y-auto" role="list">
            {/* Locked columns — shown but not toggleable */}
            {lockedColumns.map((col) => (
              <li
                key={col.id}
                className="flex items-center gap-2 px-3 py-1.5 text-sm text-text-muted select-none"
              >
                <input
                  type="checkbox"
                  checked
                  disabled
                  aria-label={`${col.label} (locked)`}
                  className="h-3.5 w-3.5 rounded border border-border"
                  readOnly
                />
                <span className="flex-1 truncate">{col.label}</span>
                <span className="text-xs">locked</span>
              </li>
            ))}

            {/* Manageable columns */}
            {manageableColumns.map((col, index) => {
              const actualIndex = columns.indexOf(col);
              const isVisible = getColumnVisibility(col.id);
              const isDragging = dragState.draggedIndex === actualIndex;
              const isTarget = dragState.targetIndex === actualIndex;

              return (
                <li
                  key={col.id}
                  draggable={isEditMode}
                  onDragStart={
                    isEditMode
                      ? (e) => handleDragStart(e as React.DragEvent<HTMLElement>, actualIndex)
                      : undefined
                  }
                  onDragOver={
                    isEditMode
                      ? (e) => handleDragOver(e as React.DragEvent<HTMLElement>, actualIndex)
                      : undefined
                  }
                  onDrop={
                    isEditMode
                      ? (e) => handleDrop(e as React.DragEvent<HTMLElement>, actualIndex)
                      : undefined
                  }
                  onDragEnd={handleDragEnd}
                  className={cn(
                    "flex items-center gap-2 px-3 py-1.5 text-sm",
                    "transition-colors duration-100",
                    isEditMode
                      ? "cursor-grab active:cursor-grabbing"
                      : "cursor-default",
                    isDragging && "opacity-40",
                    isTarget && "border-t-2 border-primary"
                  )}
                >
                  {isEditMode && (
                    <span className="shrink-0 text-text-muted">
                      <DragIcon />
                    </span>
                  )}
                  <input
                    type="checkbox"
                    id={`col-toggle-${col.id}`}
                    checked={isVisible}
                    onChange={() => toggleVisibility(col.id)}
                    aria-label={`Show ${col.label} column`}
                    className={cn(
                      "h-3.5 w-3.5 rounded border border-border shrink-0",
                      "text-primary focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                    )}
                  />
                  <label
                    htmlFor={`col-toggle-${col.id}`}
                    className={cn(
                      "flex-1 truncate cursor-pointer select-none",
                      isVisible ? "text-text-primary" : "text-text-muted"
                    )}
                  >
                    {col.label}
                  </label>
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
}
