// ============================================================
// Debounced search input hook
// Never use raw setTimeout for debounce — use this hook.
// ============================================================
'use client';
import { useState, useEffect } from 'react';
import { SEARCH_DEBOUNCE_MS } from '@/lib/constants';

/**
 * Returns a debounced copy of the input value.
 * Use the debounced value in query keys — never the raw input.
 *
 * @example
 * const [input, setInput] = useState('');
 * const debouncedSearch = useSearchDebounce(input);
 * // Use debouncedSearch in queryKey, not input
 */
export function useSearchDebounce(value: string, delay: number = SEARCH_DEBOUNCE_MS): string {
  const [debounced, setDebounced] = useState(value);

  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), delay);
    return () => clearTimeout(timer);
  }, [value, delay]);

  return debounced;
}
