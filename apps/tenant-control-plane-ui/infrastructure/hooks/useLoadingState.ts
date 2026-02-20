// ============================================================
// Coordinated loading state across concurrent operations
// Prevents multiple competing spinners.
// ============================================================
'use client';
import { useState, useCallback, useRef } from 'react';

interface UseLoadingStateReturn {
  isLoading: boolean;
  setLoading: (loading: boolean) => void;
  withLoading: <T>(fn: () => Promise<T>) => Promise<T>;
}

/**
 * Coordinates loading state across concurrent operations.
 * isLoading remains true until ALL concurrent operations complete.
 *
 * @example
 * const { isLoading, withLoading } = useLoadingState();
 * const handleExport = () => withLoading(async () => {
 *   await api.billing.exportInvoices(tenantId);
 * });
 */
export function useLoadingState(): UseLoadingStateReturn {
  const [count, setCount] = useState(0);
  const countRef = useRef(0);

  const setLoading = useCallback((loading: boolean) => {
    setCount((prev) => {
      const next = loading ? prev + 1 : Math.max(0, prev - 1);
      countRef.current = next;
      return next;
    });
  }, []);

  const withLoading = useCallback(
    async <T>(fn: () => Promise<T>): Promise<T> => {
      setLoading(true);
      try {
        return await fn();
      } finally {
        setLoading(false);
      }
    },
    [setLoading]
  );

  return {
    isLoading: count > 0,
    setLoading,
    withLoading,
  };
}
