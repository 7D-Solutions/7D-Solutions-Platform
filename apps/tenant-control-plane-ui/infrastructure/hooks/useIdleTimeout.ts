// ============================================================
// Idle timeout hook — staff console (30-min default, 5-min warning)
// Port from: docs/reference/fireproof/src/infrastructure/hooks/useIdleTimeout.ts
// Adapted: uses lib/constants instead of API settings; no apiClient dependency.
// ============================================================
'use client';
import { useEffect, useRef, useState, useCallback } from 'react';
import { IDLE_TIMEOUT_MS, IDLE_WARNING_MS } from '@/lib/constants';

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
 */
export function useIdleTimeout(options: UseIdleTimeoutOptions = {}): UseIdleTimeoutReturn {
  const { onWarning, onTimeout, enabled = true } = options;

  const [remainingMs, setRemainingMs] = useState(IDLE_TIMEOUT_MS);
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
    setRemainingMs(IDLE_TIMEOUT_MS);
    setIsWarning(false);
    warningFiredRef.current = false;
    timeoutFiredRef.current = false;
  }, []);

  const pauseTimer = useCallback(() => setIsPaused(true), []);

  const resumeTimer = useCallback(() => {
    setIsPaused(false);
    resetTimer();
  }, [resetTimer]);

  const handleActivity = useCallback(() => {
    if (!isPaused) resetTimer();
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

  // Idle checker (every 5s)
  useEffect(() => {
    if (!enabled) return;

    const check = () => {
      if (isPaused) return;
      const elapsed = Date.now() - lastActivityRef.current;
      const remaining = Math.max(0, IDLE_TIMEOUT_MS - elapsed);
      setRemainingMs(remaining);

      if (remaining <= IDLE_WARNING_MS && remaining > 0 && !warningFiredRef.current) {
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

    intervalRef.current = setInterval(check, 5000);
    check();

    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [enabled, isPaused]);

  return { remainingMs, isWarning, resetTimer, pauseTimer, resumeTimer };
}
