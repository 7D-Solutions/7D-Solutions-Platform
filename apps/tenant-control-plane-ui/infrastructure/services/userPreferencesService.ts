// ============================================================
// User Preferences Service — backend-persisted column/view preferences
// Port from: docs/reference/fireproof/src/infrastructure/services/userPreferencesService.ts
// Adapted: BFF route /api/preferences (TCP BFF pattern)
// ============================================================

interface CacheEntry<T> {
  value: T | null;
  timestamp: number;
}

class UserPreferencesService {
  private pendingSaves = new Map<string, ReturnType<typeof setTimeout>>();
  private pendingValues = new Map<string, unknown>();
  private cache = new Map<string, CacheEntry<unknown>>();
  private inFlight = new Map<string, Promise<unknown>>();
  private readonly DEBOUNCE_DELAY = 1000;
  private readonly CACHE_TTL = 5 * 60 * 1000; // 5 minutes

  async getPreference<T = unknown>(key: string, defaultValue: T | null = null): Promise<T | null> {
    const cached = this.cache.get(key);
    if (cached && Date.now() - cached.timestamp < this.CACHE_TTL) {
      return cached.value as T | null;
    }

    const inFlight = this.inFlight.get(key);
    if (inFlight) return inFlight as Promise<T | null>;

    const fetchPromise = this.fetchFromApi<T>(key, defaultValue);
    this.inFlight.set(key, fetchPromise);
    try {
      return await fetchPromise;
    } finally {
      this.inFlight.delete(key);
    }
  }

  private async fetchFromApi<T>(key: string, defaultValue: T | null): Promise<T | null> {
    try {
      const res = await fetch(`/api/preferences/${encodeURIComponent(key)}`);
      if (res.status === 404) {
        this.cache.set(key, { value: null, timestamp: Date.now() });
        return defaultValue;
      }
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json() as T;
      this.cache.set(key, { value: data, timestamp: Date.now() });
      return data;
    } catch {
      this.cache.set(key, { value: null, timestamp: Date.now() });
      return defaultValue;
    }
  }

  async savePreference(key: string, value: unknown, immediate = false): Promise<void> {
    this.cache.set(key, { value, timestamp: Date.now() });
    this.pendingValues.set(key, value);

    const existing = this.pendingSaves.get(key);
    if (existing) clearTimeout(existing);

    if (immediate) {
      await this.saveToApi(key, value);
      this.pendingValues.delete(key);
      return;
    }

    const timeout = setTimeout(async () => {
      await this.saveToApi(key, value);
      this.pendingSaves.delete(key);
      this.pendingValues.delete(key);
    }, this.DEBOUNCE_DELAY);

    this.pendingSaves.set(key, timeout);
  }

  private async saveToApi(key: string, value: unknown): Promise<void> {
    try {
      await fetch(`/api/preferences/${encodeURIComponent(key)}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ value }),
      });
    } catch {
      // Silently fail — preference save is best-effort
    }
  }

  async flushPendingSaves(): Promise<void> {
    const promises: Promise<void>[] = [];
    this.pendingSaves.forEach((timeout, key) => {
      clearTimeout(timeout);
      const value = this.pendingValues.get(key);
      if (value !== undefined) promises.push(this.saveToApi(key, value));
    });
    this.pendingSaves.clear();
    this.pendingValues.clear();
    if (promises.length > 0) await Promise.all(promises);
  }

  clearAllPending(): void {
    this.pendingSaves.forEach((t) => clearTimeout(t));
    this.pendingSaves.clear();
    this.pendingValues.clear();
    this.cache.clear();
    this.inFlight.clear();
  }

  invalidateCache(key: string): void {
    this.cache.delete(key);
  }

  clearCache(): void {
    this.cache.clear();
  }
}

export const userPreferencesService = new UserPreferencesService();

if (typeof window !== 'undefined') {
  window.addEventListener('beforeunload', () => {
    userPreferencesService.flushPendingSaves().catch(() => {});
  });
}
