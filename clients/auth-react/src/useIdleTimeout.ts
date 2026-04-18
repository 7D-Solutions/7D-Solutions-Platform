import { useEffect, useRef } from "react";

interface UseIdleTimeoutOptions {
  minutes: number;
  onTimeout: () => void;
}

const ACTIVITY_EVENTS = [
  "mousemove",
  "keydown",
  "scroll",
  "click",
  "touchstart",
] as const;

export function useIdleTimeout({ minutes, onTimeout }: UseIdleTimeoutOptions): void {
  if (minutes <= 0) throw new Error("minutes must be > 0");

  const onTimeoutRef = useRef(onTimeout);
  useEffect(() => {
    onTimeoutRef.current = onTimeout;
  });

  // minutes is in deps so the timer reschedules if the prop changes.
  // onTimeout is NOT in deps — accessed via ref so activity events never
  // trigger a React re-render (the timestamp is also ref-based for the same reason).
  useEffect(() => {
    const ms = minutes * 60 * 1000;
    const fired = { value: false };
    let timer: ReturnType<typeof setTimeout>;

    function arm() {
      timer = setTimeout(() => {
        if (!fired.value) {
          fired.value = true;
          onTimeoutRef.current();
        }
      }, ms);
    }

    function onActivity() {
      if (fired.value) return;
      clearTimeout(timer);
      arm();
    }

    arm();
    for (const ev of ACTIVITY_EVENTS) {
      window.addEventListener(ev, onActivity, { passive: true });
    }

    return () => {
      clearTimeout(timer);
      for (const ev of ACTIVITY_EVENTS) {
        window.removeEventListener(ev, onActivity);
      }
    };
  }, [minutes]);
}
