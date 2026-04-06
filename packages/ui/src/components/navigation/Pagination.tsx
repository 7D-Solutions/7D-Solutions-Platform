import React from "react";
import { cn } from "../../lib/cn.js";

export interface PaginationProps {
  page: number;
  totalPages: number;
  onPageChange: (page: number) => void;
  /** Maximum page buttons to show excluding prev/next — default 7 */
  maxVisible?: number;
  className?: string;
  "aria-label"?: string;
}

function range(start: number, end: number): number[] {
  return Array.from({ length: end - start + 1 }, (_, i) => start + i);
}

function getPageNumbers(
  current: number,
  total: number,
  maxVisible: number
): (number | "...")[] {
  if (total <= maxVisible) return range(1, total);

  const halfWing = Math.floor((maxVisible - 3) / 2);

  if (current <= halfWing + 2) {
    return [...range(1, maxVisible - 2), "...", total];
  }
  if (current >= total - halfWing - 1) {
    return [1, "...", ...range(total - maxVisible + 3, total)];
  }
  return [1, "...", ...range(current - halfWing, current + halfWing), "...", total];
}

const PrevIcon = () => (
  <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
    <polyline points="9 2 5 7 9 12" />
  </svg>
);

const NextIcon = () => (
  <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
    <polyline points="5 2 9 7 5 12" />
  </svg>
);

const btnBase = cn(
  "inline-flex items-center justify-center min-w-[2rem] h-8 px-2 rounded-md text-sm",
  "transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-1"
);

export function Pagination({
  page,
  totalPages,
  onPageChange,
  maxVisible = 7,
  className,
  "aria-label": ariaLabel = "Pagination",
}: PaginationProps) {
  if (totalPages <= 1) return null;

  const pages = getPageNumbers(page, totalPages, maxVisible);
  const canPrev = page > 1;
  const canNext = page < totalPages;

  return (
    <nav aria-label={ariaLabel} className={cn("flex items-center gap-1", className)}>
      <button
        type="button"
        onClick={() => onPageChange(page - 1)}
        disabled={!canPrev}
        aria-label="Previous page"
        className={cn(
          btnBase,
          "text-text-secondary hover:bg-gray-100 hover:text-text-primary",
          "disabled:opacity-40 disabled:cursor-not-allowed"
        )}
      >
        <PrevIcon />
      </button>

      {pages.map((p, i) =>
        p === "..." ? (
          <span
            key={`ellipsis-${i}`}
            aria-hidden="true"
            className="inline-flex items-center justify-center min-w-[2rem] h-8 px-2 text-text-muted text-sm"
          >
            …
          </span>
        ) : (
          <button
            key={p}
            type="button"
            onClick={() => onPageChange(p as number)}
            aria-label={`Page ${p}`}
            aria-current={p === page ? "page" : undefined}
            className={cn(
              btnBase,
              p === page
                ? "bg-primary text-text-inverse font-medium"
                : "text-text-secondary hover:bg-gray-100 hover:text-text-primary"
            )}
          >
            {p}
          </button>
        )
      )}

      <button
        type="button"
        onClick={() => onPageChange(page + 1)}
        disabled={!canNext}
        aria-label="Next page"
        className={cn(
          btnBase,
          "text-text-secondary hover:bg-gray-100 hover:text-text-primary",
          "disabled:opacity-40 disabled:cursor-not-allowed"
        )}
      >
        <NextIcon />
      </button>
    </nav>
  );
}
