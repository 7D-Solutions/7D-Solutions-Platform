// ============================================================
// Standardized cache invalidation hook
// Never invalidate everything — always be explicit about what to invalidate.
// ============================================================
'use client';
import { useQueryClient } from '@tanstack/react-query';
import { useCallback } from 'react';

/**
 * Explicit cache invalidation. Never use queryClient.invalidateQueries() without a key.
 *
 * @example
 * const { invalidate } = useQueryInvalidation();
 * invalidate(['tenant', tenantId]);
 * invalidate(['tenant-list']);
 */
export function useQueryInvalidation() {
  const queryClient = useQueryClient();

  const invalidate = useCallback(
    (queryKey: string[]) => {
      queryClient.invalidateQueries({ queryKey });
    },
    [queryClient]
  );

  const invalidateMany = useCallback(
    (queryKeys: string[][]) => {
      Promise.all(queryKeys.map((key) => queryClient.invalidateQueries({ queryKey: key })));
    },
    [queryClient]
  );

  return { invalidate, invalidateMany };
}
