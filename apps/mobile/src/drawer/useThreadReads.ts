import {useEffect, useState} from 'react';
import type {ThreadSummaryV1} from '../control/types';
import {readThreadReads, saveThreadRead, type ThreadReads} from '../storage/cache';

// Loads this phone's read positions for a host and keeps the open thread marked
// as seen, so only work that happened elsewhere shows as unread.
export function useThreadReads(
  hostId: string,
  threads: readonly ThreadSummaryV1[],
  selectedThreadId: string | null,
): ThreadReads | null {
  const [reads, setReads] = useState<ThreadReads | null>(null);

  useEffect(() => {
    let cancelled = false;
    setReads(null);
    if (!hostId) return;
    readThreadReads(hostId)
      .then(loaded => {
        if (!cancelled) setReads(loaded);
      })
      .catch(() => {
        // Unread markers are optional; without storage nothing is shown as unread.
      });
    return () => {
      cancelled = true;
    };
  }, [hostId]);

  const selectedUpdatedAt = threads.find(thread => thread.id === selectedThreadId)?.updated_at_ms;
  const loaded = reads !== null;
  useEffect(() => {
    if (!loaded || !hostId || !selectedThreadId || selectedUpdatedAt === undefined) return;
    setReads(current => current && (current.seen[selectedThreadId] ?? 0) < selectedUpdatedAt
      ? {...current, seen: {...current.seen, [selectedThreadId]: selectedUpdatedAt}}
      : current);
    void saveThreadRead(hostId, selectedThreadId, selectedUpdatedAt).catch(() => {});
  }, [hostId, loaded, selectedThreadId, selectedUpdatedAt]);

  return reads;
}
