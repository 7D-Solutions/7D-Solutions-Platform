'use client';
// ============================================================
// DataTable — column manager built in (show/hide, reorder, backend-persist)
// ============================================================
import { clsx } from 'clsx';
import { Settings2, RotateCcw } from 'lucide-react';
import type { UseColumnManagerReturn } from '@/infrastructure/hooks/useColumnManager';

interface DataTableProps<T extends Record<string, unknown>> {
  data: T[];
  columns: Array<{
    id: string;
    header: string;
    accessor: keyof T | ((row: T) => React.ReactNode);
    align?: 'left' | 'center' | 'right';
  }>;
  columnManager: UseColumnManagerReturn;
  keyField: keyof T;
  loading?: boolean;
  emptyMessage?: string;
  className?: string;
}

export function DataTable<T extends Record<string, unknown>>({
  data,
  columns,
  columnManager,
  keyField,
  loading = false,
  emptyMessage = 'No records found.',
  className,
}: DataTableProps<T>) {
  const { columns: managedCols, isEditMode, toggleEditMode, resetToDefault,
          handleDragStart, handleDragOver, handleDrop, handleDragEnd,
          toggleVisibility, getColumnVisibility, dragState } = columnManager;

  const visibleColumns = managedCols.filter((mc) =>
    getColumnVisibility(mc.id)
  );

  return (
    <div className={clsx('rounded-[--radius-lg] border border-[--color-border-light] overflow-hidden', className)}>
      {/* Column manager toolbar */}
      <div className="flex items-center justify-end gap-2 border-b border-[--color-border-light] bg-[--color-bg-secondary] px-4 py-2">
        {isEditMode && (
          <button
            onClick={resetToDefault}
            className="flex items-center gap-1 text-xs text-[--color-text-secondary] hover:text-[--color-text-primary] transition-[--transition-fast]"
          >
            <RotateCcw className="h-3 w-3" />
            Reset
          </button>
        )}
        <button
          onClick={toggleEditMode}
          className={clsx(
            'flex items-center gap-1 rounded px-2 py-1 text-xs transition-[--transition-fast]',
            isEditMode
              ? 'bg-[--color-primary] text-[--color-text-inverse]'
              : 'text-[--color-text-secondary] hover:bg-[--color-bg-tertiary]'
          )}
        >
          <Settings2 className="h-3 w-3" />
          {isEditMode ? 'Done' : 'Columns'}
        </button>
      </div>

      {/* Column visibility editor */}
      {isEditMode && (
        <div className="flex flex-wrap gap-2 border-b border-[--color-border-light] bg-[--color-bg-secondary] px-4 py-3">
          {managedCols.map((col) => {
            if (col.locked) return null;
            const isVisible = getColumnVisibility(col.id);
            return (
              <label key={col.id} className="flex items-center gap-1.5 cursor-pointer">
                <input
                  type="checkbox"
                  checked={isVisible}
                  onChange={() => toggleVisibility(col.id)}
                  className="h-3.5 w-3.5 accent-[--color-primary]"
                />
                <span className="text-xs text-[--color-text-primary]">{col.label}</span>
              </label>
            );
          })}
        </div>
      )}

      {/* Table */}
      <div className="overflow-x-auto">
        <table
          className="w-full border-collapse"
          style={{ fontSize: 'var(--table-body-font-size)', lineHeight: 'var(--table-line-height)' }}
        >
          <thead>
            <tr className="border-b border-[--color-border-light] bg-[--color-bg-secondary]">
              {visibleColumns.map((mc, idx) => {
                const col = columns.find((c) => c.id === mc.id);
                const align = mc.align ?? col?.align ?? 'left';
                return (
                  <th
                    key={mc.id}
                    draggable={isEditMode && !mc.locked}
                    onDragStart={isEditMode ? (e) => handleDragStart(e, idx) : undefined}
                    onDragOver={isEditMode ? (e) => handleDragOver(e, idx) : undefined}
                    onDrop={isEditMode ? (e) => handleDrop(e, idx) : undefined}
                    onDragEnd={isEditMode ? handleDragEnd : undefined}
                    className={clsx(
                      'px-[--table-cell-padding-x] py-[--table-cell-padding-y]',
                      'text-[--table-header-font-size] font-[--font-weight-semibold]',
                      'text-[--color-text-secondary] uppercase tracking-wide select-none',
                      align === 'right' && 'text-right',
                      align === 'center' && 'text-center',
                      isEditMode && !mc.locked && 'cursor-grab',
                      dragState.targetIndex === idx && 'bg-[--color-primary-lighter]'
                    )}
                  >
                    {mc.label}
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody>
            {loading ? (
              <tr>
                <td colSpan={visibleColumns.length} className="py-12 text-center text-[--color-text-muted]">
                  Loading...
                </td>
              </tr>
            ) : data.length === 0 ? (
              <tr>
                <td colSpan={visibleColumns.length} className="py-12 text-center text-[--color-text-muted]">
                  {emptyMessage}
                </td>
              </tr>
            ) : (
              data.map((row) => (
                <tr
                  key={String(row[keyField])}
                  className="border-b border-[--color-border-light] hover:bg-[--color-bg-secondary] transition-[--transition-fast]"
                  style={{ height: 'var(--table-row-height)' }}
                >
                  {visibleColumns.map((mc) => {
                    const col = columns.find((c) => c.id === mc.id);
                    const align = mc.align ?? col?.align ?? 'left';
                    const cellContent = col
                      ? typeof col.accessor === 'function'
                        ? col.accessor(row)
                        : String(row[col.accessor] ?? '')
                      : '';
                    return (
                      <td
                        key={mc.id}
                        className={clsx(
                          'px-[--table-cell-padding-x] py-[--table-cell-padding-y]',
                          align === 'right' && 'text-right',
                          align === 'center' && 'text-center'
                        )}
                      >
                        {cellContent}
                      </td>
                    );
                  })}
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
