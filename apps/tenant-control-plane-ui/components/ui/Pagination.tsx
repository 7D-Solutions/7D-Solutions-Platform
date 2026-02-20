'use client';
// ============================================================
// Pagination controls — page nav + page size selector
// ============================================================
import { clsx } from 'clsx';
import { ChevronLeft, ChevronRight } from 'lucide-react';
import { PAGINATION_MIN_PAGE_SIZE, PAGINATION_MAX_PAGE_SIZE } from '@/lib/constants';

const PAGE_SIZE_OPTIONS = [10, 25, 50, 100].filter(
  (n) => n >= PAGINATION_MIN_PAGE_SIZE && n <= PAGINATION_MAX_PAGE_SIZE,
);

interface PaginationProps {
  page: number;
  pageSize: number;
  totalCount: number;
  totalPages: number;
  hasNextPage: boolean;
  hasPrevPage: boolean;
  onNextPage: () => void;
  onPrevPage: () => void;
  onGoToPage: (page: number) => void;
  onPageSizeChange: (size: number) => void;
  className?: string;
}

export function Pagination({
  page,
  pageSize,
  totalCount,
  totalPages,
  hasNextPage,
  hasPrevPage,
  onNextPage,
  onPrevPage,
  onGoToPage,
  onPageSizeChange,
  className,
}: PaginationProps) {
  const start = totalCount === 0 ? 0 : (page - 1) * pageSize + 1;
  const end = Math.min(page * pageSize, totalCount);

  return (
    <div
      className={clsx(
        'flex items-center justify-between gap-4 px-4 py-3',
        'border-t border-[--color-border-light] bg-[--color-bg-secondary]',
        'text-sm text-[--color-text-secondary]',
        className,
      )}
      data-testid="pagination"
    >
      <div className="flex items-center gap-2">
        <span>Rows per page:</span>
        <select
          value={pageSize}
          onChange={(e) => onPageSizeChange(Number(e.target.value))}
          className="rounded border border-[--color-border-default] bg-[--color-bg-primary] px-2 py-1 text-sm"
          data-testid="page-size-select"
        >
          {PAGE_SIZE_OPTIONS.map((size) => (
            <option key={size} value={size}>
              {size}
            </option>
          ))}
        </select>
      </div>

      <div className="flex items-center gap-3">
        <span data-testid="pagination-info">
          {totalCount === 0
            ? 'No results'
            : `${start}–${end} of ${totalCount}`}
        </span>

        <div className="flex items-center gap-1">
          <button
            onClick={() => onGoToPage(1)}
            disabled={!hasPrevPage}
            className="rounded p-1 hover:bg-[--color-bg-tertiary] disabled:opacity-40 disabled:cursor-not-allowed"
            aria-label="First page"
            title="First page"
          >
            <ChevronLeft className="h-4 w-4" />
            <ChevronLeft className="h-4 w-4 -ml-3" />
          </button>
          <button
            onClick={onPrevPage}
            disabled={!hasPrevPage}
            className="rounded p-1 hover:bg-[--color-bg-tertiary] disabled:opacity-40 disabled:cursor-not-allowed"
            aria-label="Previous page"
          >
            <ChevronLeft className="h-4 w-4" />
          </button>

          <span className="px-2 tabular-nums">
            {page} / {totalPages}
          </span>

          <button
            onClick={onNextPage}
            disabled={!hasNextPage}
            className="rounded p-1 hover:bg-[--color-bg-tertiary] disabled:opacity-40 disabled:cursor-not-allowed"
            aria-label="Next page"
          >
            <ChevronRight className="h-4 w-4" />
          </button>
          <button
            onClick={() => onGoToPage(totalPages)}
            disabled={!hasNextPage}
            className="rounded p-1 hover:bg-[--color-bg-tertiary] disabled:opacity-40 disabled:cursor-not-allowed"
            aria-label="Last page"
            title="Last page"
          >
            <ChevronRight className="h-4 w-4" />
            <ChevronRight className="h-4 w-4 -ml-3" />
          </button>
        </div>
      </div>
    </div>
  );
}
