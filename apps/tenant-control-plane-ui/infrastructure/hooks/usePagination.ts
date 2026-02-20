// ============================================================
// Centralized pagination hook
// Port from: docs/reference/fireproof/src/infrastructure/hooks/usePagination.ts
// Adapted: uses next/navigation instead of react-router-dom; uses TCP constants.
// ============================================================
'use client';
import { useState, useCallback, useMemo } from 'react';
import {
  PAGINATION_DEFAULT_PAGE_SIZE,
  PAGINATION_MIN_PAGE_SIZE,
  PAGINATION_MAX_PAGE_SIZE,
} from '@/lib/constants';

interface UsePaginationOptions {
  totalCount: number;
  defaultPageSize?: number;
}

interface UsePaginationResult {
  page: number;           // 1-indexed
  pageSize: number;
  totalCount: number;
  totalPages: number;
  offset: number;
  hasNextPage: boolean;
  hasPrevPage: boolean;
  goToPage: (page: number) => void;
  nextPage: () => void;
  prevPage: () => void;
  setPageSize: (size: number) => void;
  resetPagination: () => void;
}

/**
 * Centralized pagination state.
 * Page is 1-indexed (page 1 = first page).
 * Default page size from lib/constants.ts.
 *
 * @example
 * const pagination = usePagination({ totalCount: data?.total ?? 0 });
 * // Use in query key: { page: pagination.page, pageSize: pagination.pageSize }
 */
export function usePagination({
  totalCount,
  defaultPageSize = PAGINATION_DEFAULT_PAGE_SIZE,
}: UsePaginationOptions): UsePaginationResult {
  const [page, setPage] = useState(1);
  const [pageSize, setPageSizeState] = useState(defaultPageSize);

  const totalPages = useMemo(
    () => Math.max(1, Math.ceil(totalCount / pageSize)),
    [totalCount, pageSize]
  );

  const offset = useMemo(() => (page - 1) * pageSize, [page, pageSize]);

  const goToPage = useCallback(
    (target: number) => {
      setPage(Math.max(1, Math.min(totalPages, target)));
    },
    [totalPages]
  );

  const nextPage = useCallback(() => {
    setPage((p) => Math.min(totalPages, p + 1));
  }, [totalPages]);

  const prevPage = useCallback(() => {
    setPage((p) => Math.max(1, p - 1));
  }, []);

  const setPageSize = useCallback((size: number) => {
    const clamped = Math.max(
      PAGINATION_MIN_PAGE_SIZE,
      Math.min(PAGINATION_MAX_PAGE_SIZE, size)
    );
    setPageSizeState(clamped);
    setPage(1);
  }, []);

  const resetPagination = useCallback(() => {
    setPage(1);
    setPageSizeState(defaultPageSize);
  }, [defaultPageSize]);

  return {
    page,
    pageSize,
    totalCount,
    totalPages,
    offset,
    hasNextPage: page < totalPages,
    hasPrevPage: page > 1,
    goToPage,
    nextPage,
    prevPage,
    setPageSize,
    resetPagination,
  };
}
