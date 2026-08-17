import type {
  ControlEventV1,
  JsonValue,
  PendingApprovalV1,
  TimelineItemV1,
  TimelinePageV1,
} from './types';

export interface TimelineReplica {
  threadId: string;
  epoch: string;
  firstSeq: number | null;
  lastSeq: number | null;
  hasOlder: boolean;
  hasNewer: boolean;
  items: TimelineItemV1[];
  needsTailRefresh: boolean;
}

export function controlEventInvalidatesBootstrap(eventType: string): boolean {
  return eventType.startsWith('thread.') ||
    eventType.startsWith('run.') ||
    eventType.startsWith('turn.') ||
    eventType.startsWith('approval_') ||
    eventType.startsWith('worker.') ||
    eventType === 'appearance.updated' ||
    eventType === 'models.updated' ||
    eventType === 'model_favorites.updated' ||
    eventType === 'timeline.appended' ||
    eventType === 'sync.required';
}

export function emptyReplica(threadId: string): TimelineReplica {
  return {
    threadId,
    epoch: '',
    firstSeq: null,
    lastSeq: null,
    hasOlder: false,
    hasNewer: false,
    items: [],
    needsTailRefresh: false,
  };
}

function uniqueSorted(items: TimelineItemV1[]): TimelineItemV1[] {
  const byCoordinate = new Map<string, TimelineItemV1>();
  for (const item of items) {
    byCoordinate.set(`${item.epoch}:${item.seq}`, item);
  }
  return [...byCoordinate.values()].sort((a, b) => a.seq - b.seq);
}

export function applyTimelinePage(
  current: TimelineReplica,
  page: TimelinePageV1,
  mode: 'tail' | 'after' | 'before',
): TimelineReplica {
  const epochChanged = Boolean(current.epoch && current.epoch !== page.epoch);
  const hasMiddleGap =
    mode === 'after' &&
    current.lastSeq !== null &&
    page.first_seq !== null &&
    page.first_seq > current.lastSeq + 1;
  if (epochChanged || hasMiddleGap || mode === 'tail' || !current.epoch) {
    return {
      threadId: page.thread_id,
      epoch: page.epoch,
      firstSeq: page.first_seq,
      lastSeq: page.last_seq,
      hasOlder: page.has_older,
      hasNewer: page.has_newer,
      items: uniqueSorted(page.items),
      needsTailRefresh: hasMiddleGap,
    };
  }
  const items = uniqueSorted([...current.items, ...page.items]);
  return {
    threadId: page.thread_id,
    epoch: page.epoch,
    firstSeq: items.at(0)?.seq ?? null,
    lastSeq: items.at(-1)?.seq ?? null,
    hasOlder: mode === 'before' ? page.has_older : current.hasOlder,
    hasNewer: mode === 'after' ? page.has_newer : current.hasNewer,
    items,
    needsTailRefresh: false,
  };
}

export function applyControlEvent(
  current: TimelineReplica,
  event: ControlEventV1,
  expectedHostId: string,
): TimelineReplica {
  if (event.host_id !== expectedHostId) return current;
  if (event.type === 'sync.required') {
    return {...current, needsTailRefresh: true};
  }
  if (event.type !== 'timeline.appended' || event.thread_id !== current.threadId) {
    return current;
  }
  if (current.epoch && event.epoch && current.epoch !== event.epoch) {
    return {...current, needsTailRefresh: true};
  }
  if (event.seq !== undefined && current.lastSeq !== null && event.seq > current.lastSeq + 1) {
    return {...current, needsTailRefresh: true};
  }
  const data = event.data as {item?: TimelineItemV1};
  if (!data?.item) {
    return {...current, needsTailRefresh: true};
  }
  const items = uniqueSorted([...current.items, data.item]);
  return {
    ...current,
    epoch: data.item.epoch,
    firstSeq: items.at(0)?.seq ?? null,
    lastSeq: items.at(-1)?.seq ?? null,
    items,
    needsTailRefresh: false,
  };
}

export interface ProjectedMessage {
  kind: 'message';
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  reasoning: string;
  runId: string | null;
  ledgerVersion?: number;
  seq: number;
}

export type ActivityStatus =
  | 'running'
  | 'completed'
  | 'warning'
  | 'failed'
  | 'approval';

export type ProjectedActivityIcon =
  | 'thinking'
  | 'tool'
  | 'command'
  | 'file'
  | 'worker'
  | 'image'
  | 'status';

export interface ProjectedActivityRow {
  id: string;
  kind: 'tool' | 'change' | 'status';
  seq: number;
  label: string;
  detail: string;
  status: ActivityStatus;
  icon: ProjectedActivityIcon;
  additions?: number;
  deletions?: number;
}

export interface ProjectedActivityGroup {
  kind: 'activity';
  id: string;
  runId: string;
  seq: number;
  status: ActivityStatus;
  label: string;
  detail: string;
  duration: string;
  rows: ProjectedActivityRow[];
}

export interface ProjectedApprovalItem {
  kind: 'approval';
  id: string;
  runId: string | null;
  seq: number;
  status: ActivityStatus;
  label: string;
  detail: string;
  approval: PendingApprovalV1 | null;
}

export type ProjectedTranscriptItem =
  | ProjectedMessage
  | ProjectedActivityGroup
  | ProjectedApprovalItem;

type MutableActivityGroup = {
  runId: string;
  seq: number;
  startedAtMs: number;
  completedAtMs: number | null;
  terminalStatus: ActivityStatus | null;
  rows: ProjectedActivityRow[];
  rowByKey: Map<string, ProjectedActivityRow>;
};

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function stringField(record: Record<string, unknown> | null, ...keys: string[]): string {
  for (const key of keys) {
    const value = record?.[key];
    if (typeof value === 'string' && value.trim()) return value.trim();
  }
  return '';
}

function numberField(record: Record<string, unknown> | null, ...keys: string[]): number | undefined {
  for (const key of keys) {
    const value = record?.[key];
    if (typeof value === 'number' && Number.isFinite(value)) return value;
  }
  return undefined;
}

function compactText(value: string, max = 140): string {
  const text = value.trim().replace(/\s+/g, ' ');
  return text.length > max ? `${text.slice(0, max - 1).trimEnd()}…` : text;
}

function humanize(value: string): string {
  const text = value.replaceAll('-', '_').split('_').filter(Boolean).join(' ');
  return text ? text[0].toUpperCase() + text.slice(1) : 'Activity update';
}

function parseArguments(value: unknown): Record<string, unknown> | null {
  if (typeof value !== 'string' || !value.trim()) return asRecord(value);
  try {
    return asRecord(JSON.parse(value));
  } catch {
    return null;
  }
}

function toolIcon(name: string): ProjectedActivityIcon {
  if (/^(?:shell|run_command|command)$/.test(name)) return 'command';
  if (/(?:file|patch|edit|write|read|list_dir)/.test(name)) return 'file';
  if (/(?:worker|thread)/.test(name)) return 'worker';
  if (/(?:image|screenshot|camera)/.test(name)) return 'image';
  return 'tool';
}

function toolLabel(name: string, completed: boolean, failed: boolean): string {
  const labels: Record<string, [string, string, string]> = {
    read_file: ['Reading file', 'Read file', 'Read file failed'],
    read_file_anchors: ['Reading anchored file', 'Read anchored file', 'Read anchored file failed'],
    list_dir: ['Listing files', 'Listed files', 'List files failed'],
    write_file: ['Creating file', 'Created file', 'Create file failed'],
    edit_file: ['Editing file', 'Edited file', 'Edit file failed'],
    patch_file: ['Patching file', 'Patched file', 'Patch file failed'],
    file_change: ['Changing file', 'Changed file', 'File change failed'],
    shell: ['Running command', 'Ran command', 'Command failed'],
    run_command: ['Running command', 'Ran command', 'Command failed'],
    command: ['Running command', 'Ran command', 'Command failed'],
    http_fetch: ['Fetching URL', 'Fetched URL', 'Fetch failed'],
    render_chart: ['Rendering chart', 'Rendered chart', 'Chart rendering failed'],
  };
  const known = labels[name];
  if (known) return known[failed ? 2 : completed ? 1 : 0];
  const readable = humanize(name || 'tool').toLocaleLowerCase();
  return failed
    ? `${humanize(name || 'tool')} failed`
    : completed
      ? `Used ${readable}`
      : `Using ${readable}`;
}

function toolStatus(itemType: string, data: Record<string, unknown>): ActivityStatus {
  const raw = stringField(data, 'status').toLowerCase();
  const result = asRecord(data.result);
  const failed = Boolean(stringField(data, 'error')) || Boolean(stringField(result, 'error')) ||
    ['failed', 'error', 'denied'].includes(raw);
  if (failed) return 'failed';
  if (['cancelled', 'canceled'].includes(raw)) return 'warning';
  if (itemType === 'tool_result' || itemType === 'tool_finished' || raw === 'completed') {
    return 'completed';
  }
  return 'running';
}

function toolStats(data: Record<string, unknown>): {additions?: number; deletions?: number} {
  const result = asRecord(data.result);
  return {
    additions: numberField(result, 'added', 'additions', 'insertions', 'lines_added'),
    deletions: numberField(result, 'removed', 'removals', 'deletions', 'lines_removed'),
  };
}

function toolDetail(data: Record<string, unknown>): string {
  const args = parseArguments(data.arguments);
  const result = asRecord(data.result);
  const error = stringField(data, 'error') || stringField(result, 'error');
  const path = stringField(args, 'path', 'file_path') || stringField(data, 'path', 'file_path');
  const command = stringField(args, 'command') || stringField(data, 'command');
  const url = stringField(args, 'url') || stringField(data, 'url');
  const explicit = stringField(data, 'detail');
  const {additions, deletions} = toolStats(data);
  const stats = additions !== undefined || deletions !== undefined
    ? `+${additions ?? 0} -${deletions ?? 0}`
    : '';
  if (error) return compactText(path ? `${path}: ${error}` : error);
  if (explicit) return [compactText(explicit, 320), stats].filter(Boolean).join(' ');
  if (command) return command.trim();
  if (path) return [path, stats].filter(Boolean).join(' ');
  if (url) return compactText(url, 320);
  const exitCode = numberField(result, 'exit_code');
  if (exitCode !== undefined) return `exit ${exitCode}`;
  return stats;
}

function durationLabel(startedAtMs: number, completedAtMs: number | null): string {
  if (completedAtMs === null || completedAtMs <= startedAtMs) return '';
  const seconds = Math.max(1, Math.round((completedAtMs - startedAtMs) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds % 60;
  if (minutes < 60) return remainder ? `${minutes}m ${remainder}s` : `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const minuteRemainder = minutes % 60;
  return minuteRemainder ? `${hours}h ${minuteRemainder}m` : `${hours}h`;
}

const TOOL_EVENTS = new Set([
  'tool_call',
  'tool_started',
  'tool_updated',
  'tool_finished',
  'tool_result',
]);

const APPROVAL_EVENTS = new Set([
  'approval_requested',
  'approval_status',
  'approval_resolved',
  'approval_failed',
  'tool_approval_required',
  'tool_approval_resolved',
]);

const QUIET_EVENTS = new Set([
  'start',
  'session_established',
  'turn_started',
  'usage_delta',
  'usage_updated',
  'limit_updated',
  'final',
]);

function addOrUpdateToolRow(group: MutableActivityGroup, item: TimelineItemV1, data: Record<string, unknown>) {
  const name = stringField(data, 'name') || 'tool';
  const key = stringField(data, 'id', 'call_id') || `${name}:${item.id}`;
  const status = toolStatus(item.type, data);
  const completed = status !== 'running' && status !== 'approval';
  const explicitLabel = stringField(data, 'label');
  const stats = toolStats(data);
  const next: ProjectedActivityRow = {
    id: `activity-row-${item.run_id}-${key}`,
    kind: /(?:write|edit|patch|file_change)/.test(name) ? 'change' : 'tool',
    seq: item.seq,
    label: explicitLabel || toolLabel(name, completed, status === 'failed'),
    detail: toolDetail(data),
    status,
    icon: toolIcon(name),
    ...stats,
  };
  const previous = group.rowByKey.get(key);
  if (previous) {
    const mergedDetail = next.detail && /^[+-]\d/.test(next.detail) && previous.detail
      ? `${previous.detail} ${next.detail}`
      : next.detail || previous.detail;
    Object.assign(previous, next, {
      id: previous.id,
      detail: mergedDetail,
      additions: next.additions ?? previous.additions,
      deletions: next.deletions ?? previous.deletions,
    });
    return;
  }
  group.rowByKey.set(key, next);
  group.rows.push(next);
}

function statusRow(item: TimelineItemV1, data: Record<string, unknown>): ProjectedActivityRow | null {
  const message = stringField(data, 'message', 'detail');
  if (item.type === 'runtime_notice') {
    const level = stringField(data, 'level', 'kind').toLowerCase();
    return {
      id: `activity-row-${item.id}`,
      kind: 'status',
      seq: item.seq,
      label: message || 'Runtime notice',
      detail: stringField(data, 'detail'),
      status: level === 'error' ? 'failed' : level === 'warning' ? 'warning' : 'completed',
      icon: 'status',
    };
  }
  if (item.type === 'session_recovery_required') {
    return {
      id: `activity-row-${item.id}`,
      kind: 'status',
      seq: item.seq,
      label: 'Session recovery required',
      detail: message,
      status: 'warning',
      icon: 'status',
    };
  }
  if (item.type === 'memory_registered') {
    return {
      id: `activity-row-${item.id}`,
      kind: 'status',
      seq: item.seq,
      label: 'Saved memory',
      detail: compactText(stringField(data, 'summary', 'scope_label')),
      status: 'completed',
      icon: 'tool',
    };
  }
  if (item.type === 'image_generated') {
    const rawStatus = stringField(data, 'status').toLowerCase();
    return {
      id: `activity-row-${item.id}`,
      kind: 'status',
      seq: item.seq,
      label: rawStatus === 'failed' ? 'Image generation failed' : 'Generated image',
      detail: compactText(stringField(data, 'saved_path', 'revised_prompt')),
      status: rawStatus === 'failed' ? 'failed' : 'completed',
      icon: 'image',
    };
  }
  if (/(?:worker|child_thread)/.test(item.type)) {
    const failed = /(?:error|failed)/.test(item.type) || stringField(data, 'status') === 'failed';
    const running = /(?:started|updated)/.test(item.type);
    const run = asRecord(data.run);
    const thread = asRecord(data.thread);
    return {
      id: `activity-row-${item.id}`,
      kind: 'status',
      seq: item.seq,
      label: stringField(run, 'title') || stringField(thread, 'title') || humanize(item.type),
      detail: compactText(message || stringField(data, 'operation', 'prompt')),
      status: failed ? 'failed' : running ? 'running' : 'completed',
      icon: 'worker',
    };
  }
  if (QUIET_EVENTS.has(item.type)) return null;
  const label = stringField(data, 'label', 'message', 'title');
  const detail = stringField(data, 'detail');
  if (!label && !detail) return null;
  const rawStatus = stringField(data, 'status', 'level').toLowerCase();
  return {
    id: `activity-row-${item.id}`,
    kind: 'status',
    seq: item.seq,
    label: compactText(label || humanize(item.type)),
    detail: compactText(detail),
    status: ['failed', 'error'].includes(rawStatus)
      ? 'failed'
      : rawStatus === 'warning'
        ? 'warning'
        : ['running', 'pending'].includes(rawStatus)
          ? 'running'
          : 'completed',
    icon: 'status',
  };
}

function approvalId(data: Record<string, unknown>): string {
  return stringField(data, 'approval_id', 'id');
}

function approvalRequest(item: TimelineItemV1, data: Record<string, unknown>): PendingApprovalV1 | null {
  const id = approvalId(data);
  if (!id) return null;
  const request = asRecord(data.request) ?? data;
  const requestKind = stringField(data, 'request_kind', 'kind') ||
    (stringField(data, 'name') === 'file_change' ? 'file_change' : 'command');
  return {
    id,
    run_id: item.run_id ?? '',
    thread_id: item.thread_id,
    kind: requestKind,
    request: request as JsonValue,
    status: 'pending',
    created_at_ms: item.created_at_ms,
  };
}

function approvalCopy(approval: PendingApprovalV1 | null, data: Record<string, unknown>) {
  const request = approval ? asRecord(approval.request) : data;
  const label = stringField(request, 'name', 'title', 'server_name') || 'Runtime approval';
  const detail = stringField(request, 'command', 'arguments', 'prompt', 'message', 'url') ||
    stringField(data, 'arguments');
  const notice = stringField(request, 'environment_notice');
  return {
    label: compactText(label),
    detail: compactText([detail, notice].filter(Boolean).join(' · ')),
  };
}

export function projectTranscript(
  items: TimelineItemV1[],
  pendingApprovals: PendingApprovalV1[] = [],
): ProjectedTranscriptItem[] {
  const messages: ProjectedMessage[] = [];
  const deleted = new Set<string>();
  const streaming = new Map<string, ProjectedMessage>();
  const groups = new Map<string, MutableActivityGroup>();
  const approvals = new Map<string, ProjectedApprovalItem>();
  const pendingById = new Map(pendingApprovals.map(approval => [approval.id, approval]));

  const groupFor = (item: TimelineItemV1) => {
    if (!item.run_id) return null;
    const current = groups.get(item.run_id);
    if (current) return current;
    const created: MutableActivityGroup = {
      runId: item.run_id,
      seq: item.seq,
      startedAtMs: item.created_at_ms,
      completedAtMs: null,
      terminalStatus: null,
      rows: [],
      rowByKey: new Map(),
    };
    groups.set(item.run_id, created);
    return created;
  };

  for (const item of items) {
    const data = asRecord(item.data) ?? {};
    if (item.type === 'message_deleted' && typeof data.message_id === 'string') {
      deleted.add(data.message_id);
      continue;
    }
    if (item.type === 'assistant_delta' && item.run_id) {
      const message = streaming.get(item.run_id) ?? {
        kind: 'message' as const,
        id: `stream-${item.run_id}`,
        role: 'assistant' as const,
        content: '',
        reasoning: '',
        runId: item.run_id,
        seq: item.seq,
      };
      if (typeof data.text === 'string') message.content += data.text;
      if (typeof data.reasoning === 'string') message.reasoning += data.reasoning;
      streaming.set(item.run_id, message);
      continue;
    }
    if (item.type === 'message' && typeof data.id === 'string') {
      const role = data.role;
      if (role !== 'user' && role !== 'assistant' && role !== 'system') continue;
      if (item.run_id) streaming.delete(item.run_id);
      messages.push({
        kind: 'message',
        id: data.id,
        role,
        content: typeof data.content === 'string' ? data.content : '',
        reasoning: typeof data.reasoning === 'string' ? data.reasoning : '',
        runId: item.run_id,
        ledgerVersion: typeof data.ledgerVersion === 'number' ? data.ledgerVersion : undefined,
        seq: item.seq,
      });
      continue;
    }
    if (APPROVAL_EVENTS.has(item.type)) {
      const id = approvalId(data);
      if (!id) continue;
      const existing = approvals.get(id);
      const pending = pendingById.get(id) ?? existing?.approval ?? approvalRequest(item, data);
      const decision = stringField(data, 'decision').toLowerCase();
      const failed = item.type === 'approval_failed';
      const resolved = /resolved$/.test(item.type) || decision === 'approve' || decision === 'deny';
      const copy = approvalCopy(pending, data);
      approvals.set(id, {
        kind: 'approval',
        id: `approval-${id}`,
        runId: item.run_id,
        seq: existing?.seq ?? item.seq,
        status: failed ? 'failed' : resolved ? 'completed' : 'approval',
        label: failed
          ? 'Approval failed'
          : resolved
            ? decision === 'approve' ? 'Approved' : 'Denied'
            : copy.label,
        detail: failed ? compactText(stringField(data, 'message')) : copy.detail,
        approval: resolved || failed ? null : pending,
      });
      continue;
    }
    if (item.type === 'done' || item.type === 'turn_completed') {
      const group = groupFor(item);
      if (group) {
        group.terminalStatus = 'completed';
        group.completedAtMs = item.created_at_ms;
      }
      continue;
    }
    if (item.type === 'turn_cancelled') {
      const group = groupFor(item);
      if (group) {
        group.terminalStatus = 'warning';
        group.completedAtMs = item.created_at_ms;
      }
      continue;
    }
    if (item.type === 'error' || item.type === 'turn_failed') {
      const group = groupFor(item);
      if (group) {
        group.terminalStatus = 'failed';
        group.completedAtMs = item.created_at_ms;
        const row = statusRow(item, {...data, status: 'failed', label: stringField(data, 'message') || 'Run failed'});
        if (row) group.rows.push(row);
      }
      continue;
    }
    if (TOOL_EVENTS.has(item.type)) {
      const group = groupFor(item);
      if (group) addOrUpdateToolRow(group, item, data);
      continue;
    }
    const row = statusRow(item, data);
    if (row) {
      const group = groupFor(item);
      if (group) group.rows.push(row);
    }
  }

  for (const approval of pendingApprovals) {
    if (approvals.has(approval.id)) {
      const projected = approvals.get(approval.id)!;
      projected.approval = approval;
      projected.status = 'approval';
      const copy = approvalCopy(approval, {});
      projected.label = copy.label;
      projected.detail = copy.detail;
      continue;
    }
    const copy = approvalCopy(approval, {});
    approvals.set(approval.id, {
      kind: 'approval',
      id: `approval-${approval.id}`,
      runId: approval.run_id || null,
      seq: items.at(-1)?.seq ?? 0,
      status: 'approval',
      label: copy.label,
      detail: copy.detail,
      approval,
    });
  }

  const visibleMessages = [
    ...messages.filter(message => !deleted.has(message.id)),
    ...streaming.values(),
  ];
  const answeredRuns = new Set(
    visibleMessages
      .filter(message => message.role === 'assistant' && message.runId)
      .map(message => message.runId as string),
  );
  const projectedGroups: ProjectedActivityGroup[] = [];
  for (const group of groups.values()) {
    if (!group.rows.length) continue;
    if (!group.terminalStatus && answeredRuns.has(group.runId)) {
      group.terminalStatus = 'completed';
      group.completedAtMs = Math.max(
        group.startedAtMs,
        ...items.filter(item => item.run_id === group.runId).map(item => item.created_at_ms),
      );
    }
    const orderedRows = [...group.rows].sort((a, b) => a.seq - b.seq);
    const latestRow = orderedRows.at(-1)!;
    const status = group.terminalStatus ??
      (latestRow.status === 'failed'
        ? 'failed'
        : latestRow.status === 'warning'
          ? 'warning'
          : 'running');
    const duration = durationLabel(group.startedAtMs, group.completedAtMs);
    projectedGroups.push({
      kind: 'activity',
      id: `activity-${group.runId}-${group.seq}`,
      runId: group.runId,
      seq: group.seq,
      status,
      label: status === 'failed'
        ? 'Work stopped'
        : status === 'warning'
          ? 'Work paused'
          : status === 'completed'
            ? duration ? `Worked for ${duration}` : 'Work completed'
            : latestRow.status === 'running' ? latestRow.label : 'Working…',
      detail: status === 'running'
        ? latestRow.status === 'running' ? latestRow.detail : latestRow.label
        : latestRow.label,
      duration,
      rows: orderedRows,
    });
  }

  return [...visibleMessages, ...projectedGroups, ...approvals.values()].sort(
    (a, b) => a.seq - b.seq,
  );
}

export function projectMessages(items: TimelineItemV1[]): ProjectedMessage[] {
  return projectTranscript(items).filter(
    (item): item is ProjectedMessage => item.kind === 'message',
  );
}
