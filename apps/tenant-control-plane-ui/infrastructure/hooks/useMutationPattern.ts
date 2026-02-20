// ============================================================
// Standardized mutation hook — consistent loading, errors, toasts
// Port from: docs/reference/fireproof/src/infrastructure/hooks/useMutationPattern.ts
// Adapted: uses generic Error (no custom APIError type yet)
// ============================================================
'use client';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { TOAST_DURATION_MS } from '@/lib/constants';

export interface ApiError {
  message: string;
  status?: number;
  code?: string;
}

export type ToastFn = (title: string, message: string, duration?: number) => void;

interface MutationConfig<TData, TVariables> {
  mutationFn: (variables: TVariables) => Promise<TData>;
  onSuccess?: (data: TData, variables: TVariables) => void | Promise<void>;
  onError?: (error: ApiError, variables: TVariables) => void;
  invalidateQueries?: string[][];
  successToast?: { title: string; message: string };
  errorToast?: { title: string; defaultMessage?: string };
  toastFn?: { success: ToastFn; error: ToastFn };
}

/**
 * Standardized mutation hook.
 * - Auto-invalidates queries on success
 * - Surfaces errors to caller (never swallows)
 * - Consistent loading state tracking
 *
 * @example
 * const { mutate: suspend, isPending } = useMutationPattern({
 *   mutationFn: (id: string) => api.tenants.suspend(id),
 *   successToast: { title: 'Done', message: 'Tenant suspended' },
 *   invalidateQueries: [['tenant', tenantId]],
 * });
 */
export function useMutationPattern<TData = unknown, TVariables = void>(
  config: MutationConfig<TData, TVariables>
) {
  const queryClient = useQueryClient();

  return useMutation<TData, ApiError, TVariables>({
    mutationFn: config.mutationFn,

    onSuccess: async (data, variables) => {
      if (config.successToast && config.toastFn) {
        config.toastFn.success(
          config.successToast.title,
          config.successToast.message,
          TOAST_DURATION_MS
        );
      }

      if (config.invalidateQueries) {
        await Promise.all(
          config.invalidateQueries.map((queryKey) =>
            queryClient.invalidateQueries({ queryKey })
          )
        );
      }

      await config.onSuccess?.(data, variables);
    },

    onError: (error, variables) => {
      if (config.errorToast && config.toastFn) {
        const msg = error.message || config.errorToast.defaultMessage || 'Operation failed';
        config.toastFn.error(config.errorToast.title, msg, TOAST_DURATION_MS);
      }

      config.onError?.(error, variables);
    },
  });
}
