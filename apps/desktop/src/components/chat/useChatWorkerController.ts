import { useEffect, useRef } from "react";
import type { ThreadEvent } from "../../api";

export function useChatWorkerController() {
  const childThreadEventControllersRef = useRef<Map<string, AbortController>>(
    new Map(),
  );
  const workerRunEventControllersRef = useRef<Map<string, AbortController>>(
    new Map(),
  );
  const approvedWorkerRunsRef = useRef<Set<string>>(new Set());
  const resumingWorkerRunsRef = useRef<Set<string>>(new Set());
  const workerRunReconcileRetriesRef = useRef<Map<string, number>>(new Map());
  const childThreadLiveIdsRef = useRef<Map<string, Set<string>>>(new Map());
  const childThreadEventsRef = useRef<Map<string, ThreadEvent[]>>(new Map());

  useEffect(
    () => () => {
      childThreadEventControllersRef.current.forEach((controller) =>
        controller.abort(),
      );
      childThreadEventControllersRef.current.clear();
      childThreadLiveIdsRef.current.clear();
      childThreadEventsRef.current.clear();
      workerRunEventControllersRef.current.forEach((controller) =>
        controller.abort(),
      );
      workerRunEventControllersRef.current.clear();
    },
    [],
  );

  return {
    approvedWorkerRunsRef,
    childThreadEventControllersRef,
    childThreadEventsRef,
    childThreadLiveIdsRef,
    resumingWorkerRunsRef,
    workerRunEventControllersRef,
    workerRunReconcileRetriesRef,
  };
}
