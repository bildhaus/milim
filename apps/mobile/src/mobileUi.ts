import type {ThreadSummaryV1} from './control/types';

const INBOX_GROUP_ID = '__inbox__';
const COLLAPSE_DISTANCE = 220;
const EXPAND_DISTANCE = 96;

export interface MobileThreadGroup {
  id: string;
  label: string;
  subtitle: string;
  workspace: string | null;
  threads: ThreadSummaryV1[];
  busy: boolean;
  attentionCount: number;
  updatedAtMs: number;
}

function projectName(workspace: string): string {
  const segments = workspace.split(/[\\/]/).filter(Boolean);
  return segments.at(-1) || workspace;
}

export function groupMobileThreads(
  threads: ThreadSummaryV1[],
  approvalCounts: Record<string, number>,
): MobileThreadGroup[] {
  const groups = new Map<string, MobileThreadGroup>();
  for (const thread of threads.filter(item => !item.archived_at_ms)) {
    const workspace = thread.workspace?.trim() || null;
    const id = workspace ?? INBOX_GROUP_ID;
    const group = groups.get(id) ?? {
      id,
      label: workspace ? projectName(workspace) : 'Inbox',
      subtitle: workspace ? 'Project' : 'No Project',
      workspace,
      threads: [],
      busy: false,
      attentionCount: 0,
      updatedAtMs: 0,
    };
    group.threads.push(thread);
    group.busy ||= thread.busy;
    group.attentionCount += thread.queued_turns + (approvalCounts[thread.id] ?? 0);
    group.updatedAtMs = Math.max(group.updatedAtMs, thread.updated_at_ms);
    groups.set(id, group);
  }

  return [...groups.values()]
    .map(group => ({
      ...group,
      threads: [...group.threads].sort((a, b) => b.updated_at_ms - a.updated_at_ms),
    }))
    .sort((a, b) => {
      if (a.id === INBOX_GROUP_ID) return -1;
      if (b.id === INBOX_GROUP_ID) return 1;
      return b.updatedAtMs - a.updatedAtMs || a.label.localeCompare(b.label);
    });
}

export function nextAwayFromLatest(current: boolean, distance: number): boolean {
  return current ? distance > EXPAND_DISTANCE : distance > COLLAPSE_DISTANCE;
}

export function canUseCompactComposer({
  draft,
  attachmentCount,
  inputFocused,
  pendingApproval,
  forcedOpen,
}: {
  awayFromLatest: boolean;
  draft: string;
  attachmentCount: number;
  inputFocused: boolean;
  pendingApproval: boolean;
  forcedOpen: boolean;
}): boolean {
  return !draft.trim() &&
    attachmentCount === 0 &&
    !inputFocused &&
    !pendingApproval &&
    !forcedOpen;
}

export function friendlyEndpoint(endpoint: string | null): string {
  if (!endpoint) return 'No reachable endpoint';
  try {
    const url = new URL(endpoint);
    const host = url.hostname;
    const port = url.port ? ` · Port ${url.port}` : '';
    if (host === '127.0.0.1' || host === 'localhost' || host === '10.0.2.2') {
      return `Local emulator${port}`;
    }
    if (url.protocol === 'https:' && /(?:^|\.)ts\.net$/i.test(host)) {
      return `Tailscale · ${host}${port}`;
    }
    return `${url.protocol === 'https:' ? 'Secure direct' : 'Trusted LAN'} · ${host}${port}`;
  } catch {
    return endpoint;
  }
}

export function lowercaseMilimBrand(value: string): string {
  return value.replace(/\bmilim\b/gi, 'milim');
}

export function relativeConnectionTime(timestamp: number | null, now = Date.now()): string {
  if (!timestamp) return 'Never connected';
  const seconds = Math.max(0, Math.round((now - timestamp) / 1000));
  if (seconds < 60) return 'Connected just now';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `Connected ${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `Connected ${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `Connected ${days}d ago`;
  return `Connected ${new Date(timestamp).toLocaleDateString()}`;
}
