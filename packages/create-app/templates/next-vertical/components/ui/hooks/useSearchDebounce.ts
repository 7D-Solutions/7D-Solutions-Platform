import { useEffect, useRef, useState } from "react";

export interface SearchDebounce {
  query: string;
  debouncedQuery: string;
  setQuery: (q: string) => void;
  clear: () => void;
}

export function useSearchDebounce(delay = 300): SearchDebounce {
  const [query, setQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    timerRef.current = setTimeout(() => setDebouncedQuery(query), delay);
    return () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, [query, delay]);

  const clear = () => {
    if (timerRef.current) clearTimeout(timerRef.current);
    setQuery("");
    setDebouncedQuery("");
  };

  return { query, debouncedQuery, setQuery, clear };
}
