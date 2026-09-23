import { useCallback, useEffect, useRef, useState } from "react";
import {
  getApprovalAllowances,
  revokeApprovalAllowances,
  type ApprovalAllowance,
} from "../../api";

/**
 * The active thread's "Allow for this chat" rules. Rust owns the rules; this
 * hook reads them when the thread changes or a caller asks for a refresh
 * (after a chat-scoped approval, or when the approval controls open).
 */
export function useApprovalAllowances(threadId: string) {
  const [allowances, setAllowances] = useState<ApprovalAllowance[]>([]);
  const threadRef = useRef(threadId);
  threadRef.current = threadId;

  const refresh = useCallback(() => {
    const requested = threadRef.current;
    if (!requested) return;
    void getApprovalAllowances(requested)
      .then((next) => {
        if (threadRef.current === requested) setAllowances(next);
      })
      .catch(() => {});
  }, []);

  const clear = useCallback(async (keys?: string[]) => {
    const requested = threadRef.current;
    if (!requested) return;
    const next = await revokeApprovalAllowances(requested, keys);
    if (threadRef.current === requested) setAllowances(next);
  }, []);

  useEffect(() => {
    setAllowances([]);
    refresh();
  }, [threadId, refresh]);

  return { allowances, refresh, clear };
}
