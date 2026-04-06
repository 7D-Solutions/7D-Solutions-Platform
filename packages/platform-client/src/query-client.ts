import { QueryClient } from "@tanstack/react-query";

export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: 30_000,          // 30 s
        retry: 1,
        refetchOnWindowFocus: false,
      },
      mutations: {
        retry: 0,
      },
    },
  });
}

// ---------------------------------------------------------------------------
// Key helpers
// ---------------------------------------------------------------------------

type QueryKey = readonly unknown[];

export const queryKeys = {
  all(entity: string): QueryKey {
    return [entity] as const;
  },
  list(entity: string, filters?: Record<string, unknown>): QueryKey {
    return filters ? [entity, "list", filters] : [entity, "list"];
  },
  detail(entity: string, id: string): QueryKey {
    return [entity, "detail", id] as const;
  },
  /** Tenant-scoped variants prefix keys with tenant_id for isolation. */
  tenantList(tenantId: string, entity: string, filters?: Record<string, unknown>): QueryKey {
    return filters ? [tenantId, entity, "list", filters] : [tenantId, entity, "list"];
  },
  tenantDetail(tenantId: string, entity: string, id: string): QueryKey {
    return [tenantId, entity, "detail", id] as const;
  },
};

// ---------------------------------------------------------------------------
// Invalidation helpers
// ---------------------------------------------------------------------------

export function invalidateEntity(queryClient: QueryClient, entity: string): Promise<void> {
  return queryClient.invalidateQueries({ queryKey: [entity] });
}

export function invalidateEntityDetail(
  queryClient: QueryClient,
  entity: string,
  id: string,
): Promise<void> {
  return queryClient.invalidateQueries({ queryKey: [entity, "detail", id] });
}

export function invalidateTenantEntity(
  queryClient: QueryClient,
  tenantId: string,
  entity: string,
): Promise<void> {
  return queryClient.invalidateQueries({ queryKey: [tenantId, entity] });
}
