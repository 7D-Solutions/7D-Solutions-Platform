import { useCallback } from "react";

export interface QueryClientLike {
  invalidateQueries(options: { queryKey: unknown[] }): Promise<void>;
}

let _client: QueryClientLike | null = null;

export function registerQueryClient(client: QueryClientLike): void {
  _client = client;
}

export interface QueryInvalidationResult {
  invalidate: (...keys: unknown[][]) => Promise<void>;
}

export function useQueryInvalidation(): QueryInvalidationResult {
  const invalidate = useCallback(async (...keys: unknown[][]): Promise<void> => {
    if (!_client) {
      console.warn("[useQueryInvalidation] No query client registered.");
      return;
    }
    await Promise.all(keys.map((key) => _client!.invalidateQueries({ queryKey: key })));
  }, []);

  return { invalidate };
}
