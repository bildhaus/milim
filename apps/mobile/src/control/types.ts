export const CONTROL_PROTOCOL_VERSION = 1;

export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | {[key: string]: JsonValue};

export interface ProtocolRangeV1 {
  min: number;
  max: number;
}

export interface ControlCapabilitiesV1 {
  timeline_sync: boolean;
  queued_turns: boolean;
  approvals: boolean;
  agents: boolean;
  workers: boolean;
  attachments: boolean;
  websocket_tickets: boolean;
  lan_discovery: boolean;
  push_notifications: boolean;
  inline_branches: boolean;
  appearance_assets?: boolean;
  run_ledger?: boolean;
  run_inspection?: boolean;
  effective_run_preview?: boolean;
  steering?: boolean;
  context_injection?: boolean;
  model_favorites?: boolean;
  thread_links?: boolean;
  thread_origins?: boolean;
}

export interface ThreadLinkV1 {
  owner_thread_id: string;
  target_thread_id: string;
  target_title: string;
  target_workspace: string | null;
  target_project: string | null;
  target_model: string | null;
  target_runtime: string;
  target_archived_at_ms: number | null;
  target_busy: boolean;
  target_queued_turns: number;
  created_at_ms: number;
}

export interface FrozenLinkedThreadGrantV1 {
  target_thread_id: string;
  title: string;
  workspace: string | null;
  project: string | null;
  model: string | null;
  runtime: string;
  revision: number;
  epoch: string;
  max_timeline_seq: number;
}

export interface MailboxOriginV1 {
  exchange_id: string;
  origin_thread_id: string;
  origin_title: string;
  origin_workspace: string | null;
  origin_project: string | null;
}

export type ThreadOriginV1 = {
  kind: 'schedule';
  schedule_id: string;
  schedule_name: string;
  occurrence_unix: number;
};

export interface ThreadSummaryV1 {
  id: string;
  title: string;
  revision: number;
  epoch: string;
  updated_at_ms: number;
  archived_at_ms: number | null;
  model: string | null;
  reasoning_effort_overrides?: Record<string, string>;
  agent_id: string | null;
  workspace: string | null;
  origin?: ThreadOriginV1;
  busy: boolean;
  queued_turns: number;
  linked_threads?: ThreadLinkV1[];
}

export interface AgentSummaryV1 {
  id: string;
  name: string;
  description: string;
  avatar: string;
  tool_mode: string;
  enabled_tool_count: number;
  skill_mode: string;
  enabled_skill_count: number;
}

export interface ControlAttachmentV1 {
  id: string;
  name: string;
  mime: string;
  size: number;
  content?: string;
  data_url?: string;
  truncated?: boolean;
  /** Client-only path removed after send/cancel/cleanup. */
  local_uri?: string;
}

export interface FrozenRunConfigV1 {
  model: string;
  global_instructions: string;
  instructions: string;
  workspace: string | null;
  privacy: string;
  approval_mode: string;
  plan_mode: boolean;
  sandbox: boolean;
  computer_use: boolean;
  memory: boolean;
  delegation_policy: string;
  worker_model: string;
  agent: JsonValue;
  tool_mode: string;
  enabled_tools: string[];
  skill_mode: string;
  enabled_skills: string[];
  attachments: ControlAttachmentV1[];
  native_session_id: string | null;
  reasoning_effort: string | null;
  adapter: string;
  linked_thread_grants?: FrozenLinkedThreadGrantV1[];
  claimed_mailbox_ids?: string[];
}

export interface RunSnapshotV1 {
  id: string;
  thread_id: string;
  status: string;
  adapter: string;
  config: FrozenRunConfigV1;
  capabilities: {
    ledger: boolean;
    inspectable: boolean;
    steering: boolean;
    visibility: string;
  };
  created_at_ms: number;
  updated_at_ms: number;
  completed_at_ms: number | null;
  error: JsonValue;
}

export interface PendingApprovalV1 {
  id: string;
  run_id: string;
  thread_id: string;
  kind: string;
  request: JsonValue;
  status: string;
  created_at_ms: number;
}

export interface QueuedTurnV1 {
  id: string;
  thread_id: string;
  command_id: string;
  accepted_at_ms: number;
  display_text: string;
  attachments: ControlAttachmentV1[];
  mailbox_origin?: MailboxOriginV1;
}

export interface PendingInputV1 {
  id: string;
  thread_id: string;
  target_run_id: string | null;
  kind: string;
  state: string;
  display_text?: string;
  attachments?: ControlAttachmentV1[];
  created_at_ms: number;
}

export interface AppearanceSnapshotV1 {
  revision: string;
  theme_id: string;
  name: string;
  is_dark: boolean;
  colors: {
    primary_text: string;
    secondary_text: string;
    tertiary_text: string;
    placeholder_text: string;
    bg_primary: string;
    bg_secondary: string;
    bg_tertiary: string;
    sidebar_bg: string;
    accent: string;
    accent_light: string;
    border_primary: string;
    border_secondary: string;
    focus_border: string;
    success: string;
    warning: string;
    error: string;
    info: string;
    card_bg: string;
    card_border: string;
    input_bg: string;
    input_border: string;
  };
  glass: {
    enabled: boolean;
    blur_radius: number;
    opacity_primary: number;
    opacity_secondary: number;
    edge_light: string;
  };
  background: {
    has_image: boolean;
    image_opacity: number;
    image_blur: number;
    overlay_color: string | null;
    overlay_opacity: number;
    fit: 'cover' | 'contain' | 'tile' | 'center';
    treatment: 'clear' | 'dim' | 'blur' | 'mono';
  };
  borders: {
    card_radius: number;
    input_radius: number;
    border_opacity: number;
  };
  typography: {
    font_family: string;
    mono_family: string;
  };
}

export interface ControlBootstrapV1 {
  protocol: ProtocolRangeV1;
  host_id: string;
  host_name: string;
  capabilities: ControlCapabilitiesV1;
  appearance?: AppearanceSnapshotV1;
  threads: ThreadSummaryV1[];
  models: JsonValue[];
  favorite_model_ids?: string[];
  agents: AgentSummaryV1[];
  active_runs: RunSnapshotV1[];
  queued_turns: QueuedTurnV1[];
  pending_inputs: PendingInputV1[];
  pending_approvals: PendingApprovalV1[];
}

export interface TimelineItemV1 {
  id: string;
  thread_id: string;
  epoch: string;
  seq: number;
  run_id: string | null;
  type: string;
  data: JsonValue;
  created_at_ms: number;
}

export interface TimelinePageV1 {
  thread_id: string;
  epoch: string;
  first_seq: number | null;
  last_seq: number | null;
  has_older: boolean;
  has_newer: boolean;
  before_seq: number | null;
  after_seq: number | null;
  items: TimelineItemV1[];
}

export interface ControlEventV1 {
  event_id: string;
  host_id: string;
  thread_id?: string;
  epoch?: string;
  seq?: number;
  type: string;
  data: JsonValue;
}

export type ControlCommandKindV1 =
  | 'thread.create'
  | 'thread.rename'
  | 'thread.archive'
  | 'thread.delete'
  | 'thread.set_model'
  | 'thread.set_agent'
  | 'message.delete'
  | 'model_favorites.set'
  | 'turn.send'
  | 'turn.steer'
  | 'context.inject'
  | 'turn.inbox_delete'
  | 'turn.stop'
  | 'turn.regenerate'
  | 'turn.queue_resume'
  | 'turn.queue_move'
  | 'turn.queue_delete'
  | 'approval.resolve'
  | 'worker.start'
  | 'worker.continue_solo'
  | 'worker.stop';

export interface ControlCommandV1 {
  command_id: string;
  kind: ControlCommandKindV1;
  thread_id?: string;
  expected_revision?: number;
  payload?: JsonValue;
  confirmation_token?: string;
}

export type ControlCommandStatusV1 =
  | 'applied'
  | 'accepted'
  | 'queued'
  | 'needs_confirmation'
  | 'conflict'
  | 'failed';

export interface ControlCommandResultV1 {
  command_id: string;
  status: ControlCommandStatusV1;
  thread_id?: string;
  revision?: number;
  run_id?: string;
  queue_id?: string;
  confirmation_token?: string;
  message?: string;
  data: JsonValue;
}

export interface SavedHost {
  hostId: string;
  displayName: string;
  protocol: ProtocolRangeV1;
  candidates: string[];
  lastSuccessfulUrl: string | null;
  lastConnectedAt: number | null;
}

export interface PairedCredential {
  device_id: string;
  device_key: string;
  device_name: string;
}

export interface PairingRequestCreated {
  request_id: string;
  request_key: string;
  expires_at: number;
}

export interface PairingRequestStatus {
  request_id: string;
  status: 'pending' | 'approved' | 'denied' | 'paired';
  expires_at: number;
}

export interface MobileHostProbe {
  service: 'milim-mobile-control';
  host_id: string;
  host_name: string;
  protocol: ProtocolRangeV1;
}

export function isProtocolCompatible(range: ProtocolRangeV1): boolean {
  return range.min <= CONTROL_PROTOCOL_VERSION && range.max >= CONTROL_PROTOCOL_VERSION;
}
