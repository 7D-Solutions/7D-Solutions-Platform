// Exported for testing — lets tests verify exactly-once refresh semantics.
export interface RefreshDeduplicator {
  run(refreshFn: () => Promise<string>): Promise<string>;
  getPending(): Promise<string> | null;
}

export function createRefreshDeduplicator(): RefreshDeduplicator {
  let pending: Promise<string> | null = null;
  return {
    run(refreshFn: () => Promise<string>): Promise<string> {
      if (!pending) {
        pending = refreshFn().finally(() => {
          pending = null;
        });
      }
      return pending;
    },
    getPending() {
      return pending;
    },
  };
}
