import { useCallback, useRef, useState } from "react";

export interface MutationConfig<TData, TVariables> {
  mutationFn: (variables: TVariables) => Promise<TData>;
  onSuccess?: (data: TData, variables: TVariables) => void | Promise<void>;
  onError?: (error: Error, variables: TVariables) => void;
}

export interface MutationResult<TData, TVariables> {
  mutate: (variables: TVariables) => void;
  mutateAsync: (variables: TVariables) => Promise<TData>;
  isPending: boolean;
  error: Error | null;
  data: TData | undefined;
  reset: () => void;
}

export function useMutationPattern<TData = unknown, TVariables = void>(
  config: MutationConfig<TData, TVariables>
): MutationResult<TData, TVariables> {
  const [isPending, setIsPending] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  const [data, setData] = useState<TData | undefined>(undefined);

  const configRef = useRef(config);
  configRef.current = config;

  const mutateAsync = useCallback(async (variables: TVariables): Promise<TData> => {
    setIsPending(true);
    setError(null);
    try {
      const result = await configRef.current.mutationFn(variables);
      setData(result);
      await configRef.current.onSuccess?.(result, variables);
      return result;
    } catch (err) {
      const e = err instanceof Error ? err : new Error(String(err));
      setError(e);
      configRef.current.onError?.(e, variables);
      throw e;
    } finally {
      setIsPending(false);
    }
  }, []);

  const mutate = useCallback(
    (variables: TVariables) => {
      mutateAsync(variables).catch(() => {});
    },
    [mutateAsync]
  );

  const reset = useCallback(() => {
    setError(null);
    setData(undefined);
  }, []);

  return { mutate, mutateAsync, isPending, error, data, reset };
}
