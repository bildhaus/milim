export type MilimLink =
  | {kind: 'pair'; claim: string}
  | {kind: 'thread'; threadId: string; hostId: string | null};

// milim://thread/<thread id>?host_id=<host id> opens a thread on a paired
// desktop. Every other link is handed to pairing, which validates it itself.
export function parseMilimLink(value: string): MilimLink | null {
  const text = value.trim();
  if (!text) return null;
  const thread = /^milim:\/\/thread\/([^/?#]+)(?:\?([^#]*))?/i.exec(text);
  if (!thread) return {kind: 'pair', claim: text};
  let threadId: string;
  try {
    threadId = decodeURIComponent(thread[1]);
  } catch {
    return null;
  }
  const hostId = new URLSearchParams(thread[2] ?? '').get('host_id')?.trim() || null;
  return threadId ? {kind: 'thread', threadId, hostId} : null;
}

export function threadLink(hostId: string, threadId: string): string {
  return `milim://thread/${encodeURIComponent(threadId)}?host_id=${encodeURIComponent(hostId)}`;
}
