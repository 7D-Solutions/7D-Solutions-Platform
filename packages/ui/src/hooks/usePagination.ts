import { useCallback, useMemo, useState } from "react";

export interface PaginationResult {
  page: number;
  totalPages: number;
  pageSize: number;
  setPage: (page: number) => void;
  next: () => void;
  prev: () => void;
  canNext: boolean;
  canPrev: boolean;
  /** 1-based index of the first item on the current page */
  pageStart: number;
  /** 1-based index of the last item on the current page */
  pageEnd: number;
}

export function usePagination(
  total: number,
  pageSize: number,
  initialPage = 1
): PaginationResult {
  const [page, setPageRaw] = useState(initialPage);

  const totalPages = useMemo(
    () => Math.max(1, Math.ceil(total / pageSize)),
    [total, pageSize]
  );

  const setPage = useCallback(
    (p: number) => setPageRaw(Math.max(1, Math.min(p, totalPages))),
    [totalPages]
  );

  const next = useCallback(() => setPage(page + 1), [page, setPage]);
  const prev = useCallback(() => setPage(page - 1), [page, setPage]);

  const canNext = page < totalPages;
  const canPrev = page > 1;
  const pageStart = total === 0 ? 0 : (page - 1) * pageSize + 1;
  const pageEnd = Math.min(page * pageSize, total);

  return { page, totalPages, pageSize, setPage, next, prev, canNext, canPrev, pageStart, pageEnd };
}
