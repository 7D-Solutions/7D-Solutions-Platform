import { useCallback } from "react";

/**
 * Minimal interface compatible with TanStack QueryClient and any other query client.
 */
export interface QueryClientLike {
  invalidateQueries(options: { queryKey: unknown[] }): Promise<void>;
}

let _client: QueryClientLike | null = null;

/**
 * Register the query client used by useQueryInvalidation.
 * Call once at app initialisation, before any invalidation hooks run.
 *
 * @example — TanStack Query
 *   import { QueryClient } from "@tanstack/react-query";
 *   import { registerQueryClient } from "@7d/ui";
 *   const queryClient = new QueryClient();
 *   registerQueryClient(queryClient);
 */
export function registerQueryClient(client: QueryClientLike): void {
  _client = client;
}

export interface QueryInvalidationResult {
  /**
   * Invalidate one or more query keys in parallel.
   * Always pass explicit keys — never use a wildcard to invalidate everything.
   *
   * @example
   *   const { invalidate } = useQueryInvalidation();
   *   await invalidate(['tenants'], ['tenant', tenantId]);
   */
  invalidate: (...keys: unknown[][]) => Promise<void>;
}

/**
 * Generic query invalidation hook.
 *
 * Requires a query client to be registered first via `registerQueryClient`.
 * For apps using TanStack Query directly, wrap with:
 *
 * @example — TanStack-specific override (copy to your app's hooks/)
 *   import { useQueryClient } from "@tanstack/react-query";
 *   export function useQueryInvalidation() {
 *     const qc = useQueryClient();
 *     const invalidate = async (...keys: unknown[][]) => {
 *       await Promise.all(keys.map(k => qc.invalidateQueries({ queryKey: k })));
 *     };
 *     return { invalidate };
 *   }
 */
export function useQueryInvalidation(): QueryInvalidationResult {
  const invalidate = useCallback(async (...keys: unknown[][]): Promise<void> => {
    if (!_client) {
      console.warn(
        "[useQueryInvalidation] No query client registered. Call registerQueryClient() at app initialisation."
      );
      return;
    }
    await Promise.all(keys.map((key) => _client!.invalidateQueries({ queryKey: key })));
  }, []);

  return { invalidate };
}
