import { useEffect, useState } from "react";

/**
 * Seconds left before a rate-limited request may be retried. The countdown
 * restarts whenever `key` changes (a new notice) and settles at 0.
 */
export function useRetryCountdown(seconds: number | undefined, key: unknown): number {
  const [remaining, setRemaining] = useState(0);
  useEffect(() => {
    const total = seconds && seconds > 0 ? Math.ceil(seconds) : 0;
    setRemaining(total);
    if (!total) return;
    const deadline = Date.now() + total * 1000;
    const timer = window.setInterval(() => {
      const next = Math.max(0, Math.ceil((deadline - Date.now()) / 1000));
      setRemaining(next);
      if (next === 0) window.clearInterval(timer);
    }, 1000);
    return () => window.clearInterval(timer);
  }, [seconds, key]);
  return remaining;
}
