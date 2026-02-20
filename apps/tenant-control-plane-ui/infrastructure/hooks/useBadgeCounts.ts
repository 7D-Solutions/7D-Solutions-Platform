// ============================================================
// Nav badge counts hook — staff console
// Returns badge counts for left-nav items.
// Data sources defined in TCP-UI-VISION.md.
// ============================================================
'use client';
import { useQuery } from '@tanstack/react-query';
import { REFETCH_INTERVAL_MS } from '@/lib/constants';

export interface BadgeCounts {
  tenants?: number;      // Active provisioning jobs
  billing?: number;      // Overdue invoices
  support?: number;      // Open support sessions
  [key: string]: number | undefined;
}

async function fetchBadgeCounts(): Promise<BadgeCounts> {
  const res = await fetch('/api/badge-counts');
  if (!res.ok) return {};
  return res.json();
}

/**
 * Returns badge counts for left-nav items.
 * Polls every REFETCH_INTERVAL_MS (30s default).
 *
 * @example
 * const counts = useBadgeCounts();
 * // counts.billing → number of overdue invoices
 */
export function useBadgeCounts(): BadgeCounts {
  const { data } = useQuery<BadgeCounts>({
    queryKey: ['badge-counts'],
    queryFn: fetchBadgeCounts,
    refetchInterval: REFETCH_INTERVAL_MS,
    staleTime: REFETCH_INTERVAL_MS / 2,
    retry: false,
  });

  return data ?? {};
}
