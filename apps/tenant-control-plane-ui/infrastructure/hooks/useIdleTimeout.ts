// ============================================================
// Idle timeout hook — staff console (30-min default, 5-min warning)
// Port from: docs/reference/fireproof/src/infrastructure/hooks/useIdleTimeout.ts
// Adapted: uses lib/constants instead of API settings; no apiClient dependency.
//
// Test overrides: set window.__TCP_IDLE_MS / window.__TCP_IDLE_WARN_MS
// via Playwright addInitScript to use shortened durations.
// ============================================================
'use client';
import { useEffect, useRef, useState, useCallback, useMemo } from 'react';
import { IDLE_TIMEOUT_MS, IDLE_WARNING_MS } from '@/lib/constants';

/** Window-level test overrides (set by Playwright addInitScript) */
interface WindowWithIdleOverrides {
  __TCP_IDLE_MS?: number;
  __TCP_IDLE_WARN_MS?: number;
}

function getIdleConfig(): { timeoutMs: number; warningMs: number } {
  if (typeof window !== 'undefined') {
    const w = window as unknown as WindowWithIdleOverrides;
    const timeout = typeof w.__TCP_IDLE_MS === 'number' ? w.__TCP_IDLE_MS : IDLE_TIMEOUT_MS;
    const warning = typeof w.__TCP_IDLE_WARN_MS === 'number' ? w.__TCP_IDLE_WARN_MS : IDLE_WARNING_MS;
    return { timeoutMs: timeout, warningMs: warning };
  }
  return { timeoutMs: IDLE_TIMEOUT_MS, warningMs: IDLE_WARNING_MS };
}

export interface UseIdleTimeoutOptions {
  onWarning?: () => void;
  onTimeout?: () => void;
  enabled?: boolean;
}

export interface UseIdleTimeoutReturn {
  remainingMs: number;
  isWarning: boolean;
  resetTimer: () => void;
  pauseTimer: () => void;
  resumeTimer: () => void;
}

/**
 * Tracks user inactivity and fires onWarning / onTimeout callbacks.
 * Default: 30-minute timeout with a 5-minute warning window.
 * Activity events: mousemove, keydown, click, scroll, touchstart.
 *
 * Activity resets are suppressed while the warning is active — only the
 * explicit "Stay logged in" action (calling resetTimer) dismisses it.
 */
export function useIdleTimeout(options: UseIdleTimeoutOptions = {}): UseIdleTimeoutReturn {
  const { onWarning, onTimeout, enabled = true } = options;

  // Read config once on mount (supports test overrides via window globals)
  const { timeoutMs, warningMs } = useMemo(() => getIdleConfig(), []);
  // Adaptive check interval: 1s for short timeouts (tests), 5s for production
  const checkIntervalMs = timeoutMs < 60_000 ? 1000 : 5000;

  const [remainingMs, setRemainingMs] = useState(timeoutMs);
  const [isWarning, setIsWarning] = useState(false);
  const [isPaused, setIsPaused] = useState(false);

  const lastActivityRef = useRef(Date.now());
  const warningFiredRef = useRef(false);
  const timeoutFiredRef = useRef(false);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Store callbacks in refs to avoid effect re-runs when caller
  // passes inline arrow functions (which change identity every render).
  const onWarningRef = useRef(onWarning);
  const onTimeoutRef = useRef(onTimeout);
  useEffect(() => { onWarningRef.current = onWarning; }, [onWarning]);
  useEffect(() => { onTimeoutRef.current = onTimeout; }, [onTimeout]);

  const resetTimer = useCallback(() => {
    lastActivityRef.current = Date.now();
    setRemainingMs(timeoutMs);
    setIsWarning(false);
    warningFiredRef.current = false;
    timeoutFiredRef.current = false;
  }, [timeoutMs]);

  const pauseTimer = useCallback(() => setIsPaused(true), []);

  const resumeTimer = useCallback(() => {
    setIsPaused(false);
    resetTimer();
  }, [resetTimer]);

  // Activity resets are suppressed once the warning modal is active.
  // Only the explicit resetTimer() call (from "Stay logged in") resets.
  const handleActivity = useCallback(() => {
    if (!isPaused && !warningFiredRef.current) resetTimer();
  }, [isPaused, resetTimer]);

  // Activity listeners (throttled to 1s)
  useEffect(() => {
    if (!enabled) return;

    const events = ['mousemove', 'keydown', 'click', 'scroll', 'touchstart'] as const;
    let throttle: ReturnType<typeof setTimeout> | null = null;
    const throttled = () => {
      if (!throttle) {
        handleActivity();
        throttle = setTimeout(() => { throttle = null; }, 1000);
      }
    };

    events.forEach((e) => window.addEventListener(e, throttled));
    return () => {
      events.forEach((e) => window.removeEventListener(e, throttled));
      if (throttle) clearTimeout(throttle);
    };
  }, [handleActivity, enabled]);

  // Idle checker
  useEffect(() => {
    if (!enabled) return;

    const check = () => {
      if (isPaused) return;
      const elapsed = Date.now() - lastActivityRef.current;
      const remaining = Math.max(0, timeoutMs - elapsed);
      setRemainingMs(remaining);

      if (remaining <= warningMs && remaining > 0 && !warningFiredRef.current) {
        warningFiredRef.current = true;
        setIsWarning(true);
        onWarningRef.current?.();
      }

      if (remaining <= 0 && !timeoutFiredRef.current) {
        timeoutFiredRef.current = true;
        setIsWarning(false);
        onTimeoutRef.current?.();
      }
    };

    intervalRef.current = setInterval(check, checkIntervalMs);
    check();

    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [enabled, isPaused, timeoutMs, warningMs, checkIntervalMs]);

  return { remainingMs, isWarning, resetTimer, pauseTimer, resumeTimer };
}
