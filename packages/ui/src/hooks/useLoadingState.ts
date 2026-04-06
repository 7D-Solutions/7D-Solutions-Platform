import { useCallback, useState } from "react";

export interface LoadingState {
  loading: boolean;
  setLoading: (loading: boolean) => void;
  /** Wraps an async function — sets loading=true before and false after */
  wrap: <T>(fn: () => Promise<T>) => Promise<T>;
}

export function useLoadingState(initial = false): LoadingState {
  const [loading, setLoading] = useState(initial);

  const wrap = useCallback(async <T>(fn: () => Promise<T>): Promise<T> => {
    setLoading(true);
    try {
      return await fn();
    } finally {
      setLoading(false);
    }
  }, []);

  return { loading, setLoading, wrap };
}
