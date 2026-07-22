import { useEffect, useRef } from "react";
import { useUpdateStore } from "../update/store";

export function AutoUpdater() {
  const runningRef = useRef(false);
  const checkNow = useUpdateStore((s) => s.checkNow);
  const automaticCheck = useUpdateStore((s) => s.automaticCheck);

  useEffect(() => {
    async function run(startup = false) {
      if (runningRef.current) return;
      runningRef.current = true;
      try {
        await checkNow({ automatic: true, startup });
      } catch (error) {
        console.warn("Auto-update check failed:", error);
      } finally {
        runningRef.current = false;
      }
    }

    if (!automaticCheck) return;
    void run(true);
    const timer = window.setInterval(() => void run(), 60 * 60 * 1000);
    return () => {
      window.clearInterval(timer);
    };
  }, [automaticCheck, checkNow]);

  return null;
}
