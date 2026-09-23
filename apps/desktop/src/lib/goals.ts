import type { ChatMessage } from "../api";

export type GoalStatus =
  | "idle"
  | "running"
  | "waiting_for_worker_approval"
  | "paused"
  | "complete"
  | "blocked"
  | "error";
export type GoalDecisionStatus = "continue" | "complete" | "blocked";

export interface GoalSettings {
  objective: string;
  successCriteria: string;
  constraints: string;
  status: GoalStatus;
  lastReason: string;
  nextPrompt: string;
  turns: number;
  startedAt: number | null;
  updatedAt: number | null;
  lastSeenAt: number | null;
  developerMaxTurns: number | null;
}

export interface GoalDecision {
  status: GoalDecisionStatus;
  reason: string;
  next: string;
}

export const DEFAULT_GOAL_SETTINGS: GoalSettings = {
  objective: "",
  successCriteria: "",
  constraints: "",
  status: "idle",
  lastReason: "",
  nextPrompt: "",
  turns: 0,
  startedAt: null,
  updatedAt: null,
  lastSeenAt: null,
  developerMaxTurns: null,
};

const GOAL_STATUSES: GoalStatus[] = [
  "idle",
  "running",
  "waiting_for_worker_approval",
  "paused",
  "complete",
  "blocked",
  "error",
];
const DECISION_STATUSES: GoalDecisionStatus[] = ["continue", "complete", "blocked"];

function stringValue(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function nullableTimestamp(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function maxTurnsValue(value: unknown): number | null {
  const numeric = typeof value === "number"
    ? value
    : typeof value === "string" && value.trim()
      ? Number(value)
      : null;
  return typeof numeric === "number" && Number.isFinite(numeric) && numeric > 0
    ? Math.floor(numeric)
    : null;
}

export function normalizeGoalSettings(
  value?: Partial<GoalSettings> | null,
  options: { pauseRunning?: boolean } = {},
): GoalSettings {
  const raw = value ?? {};
  const status = GOAL_STATUSES.includes(raw.status as GoalStatus) ? raw.status as GoalStatus : DEFAULT_GOAL_SETTINGS.status;
  return {
    objective: stringValue(raw.objective),
    successCriteria: stringValue(raw.successCriteria),
    constraints: stringValue(raw.constraints),
    status: options.pauseRunning && status === "running" ? "paused" : status,
    lastReason: stringValue(raw.lastReason),
    nextPrompt: stringValue(raw.nextPrompt),
    turns: typeof raw.turns === "number" && Number.isFinite(raw.turns) && raw.turns > 0 ? Math.floor(raw.turns) : 0,
    startedAt: nullableTimestamp(raw.startedAt),
    updatedAt: nullableTimestamp(raw.updatedAt),
    lastSeenAt: nullableTimestamp(raw.lastSeenAt),
    developerMaxTurns: maxTurnsValue(raw.developerMaxTurns),
  };
}

export function goalConfigured(goal: GoalSettings): boolean {
  return goal.objective.trim().length > 0;
}

export function goalChipVisible(goal: GoalSettings): boolean {
  const completeUnread = goal.status === "complete" && (goal.updatedAt ?? 0) > (goal.lastSeenAt ?? 0);
  return goal.status === "running" || goal.status === "waiting_for_worker_approval" || goal.status === "paused" || goal.status === "blocked" || goal.status === "error" || completeUnread;
}

export function goalInstructionMessage(goal: GoalSettings): ChatMessage | null {
  const objective = goal.objective.trim();
  if (!objective) return null;
  const lines = [
    "Active autonomous goal for this thread.",
    `Objective: ${objective}`,
    goal.successCriteria.trim() ? `Success criteria: ${goal.successCriteria.trim()}` : "",
    goal.constraints.trim() ? `Constraints: ${goal.constraints.trim()}` : "",
    "Work toward this goal in the visible assistant turn. Do not output goal-controller JSON in the visible reply.",
  ].filter(Boolean);
  return { role: "system", content: lines.join("\n") };
}

function clip(text: string, limit: number): string {
  const trimmed = text.trim();
  if (trimmed.length <= limit) return trimmed;
  return `${trimmed.slice(0, limit - 3)}...`;
}

function transcriptForDecision(messages: ChatMessage[]): string {
  return messages
    .slice(-10)
    .map((message) => {
      const role = message.role === "assistant" ? "Assistant" : message.role === "system" ? "System" : "User";
      return `${role}: ${clip(message.content, 1200)}`;
    })
    .join("\n\n");
}

export function goalDecisionMessages(goal: GoalSettings, messages: ChatMessage[]): ChatMessage[] {
  return [
    {
      role: "system",
      content: [
        "You are milim's autonomous goal controller.",
        "Decide whether the active goal should continue, complete, or stop as blocked.",
        "Return only strict JSON with exactly these keys: status, reason, next.",
        "status must be one of: continue, complete, blocked.",
        "For continue, next is the next synthetic user turn. For complete or blocked, next may be empty.",
      ].join("\n"),
    },
    {
      role: "user",
      content: [
        `Objective: ${goal.objective.trim()}`,
        goal.successCriteria.trim() ? `Success criteria: ${goal.successCriteria.trim()}` : "",
        goal.constraints.trim() ? `Constraints: ${goal.constraints.trim()}` : "",
        "",
        "Recent transcript:",
        transcriptForDecision(messages),
        "",
        'Return JSON like {"status":"continue","reason":"...","next":"..."}',
      ].filter(Boolean).join("\n"),
    },
  ];
}

export function goalContinuationPrompt(goal: GoalSettings, next: string): string {
  return next.trim() || `Continue working toward the active goal: ${goal.objective.trim()}`;
}

function extractGoalDecisionJson(raw: string): string {
  const trimmed = raw.trim();
  const fence = trimmed.match(/```(?:json)?\s*([\s\S]*?)```/i);
  if (fence) return extractGoalDecisionJson(fence[1]);
  const start = trimmed.indexOf("{");
  if (start < 0) return trimmed;

  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let i = start; i < trimmed.length; i += 1) {
    const ch = trimmed[i];
    if (inString) {
      if (escaped) escaped = false;
      else if (ch === "\\") escaped = true;
      else if (ch === "\"") inString = false;
      continue;
    }
    if (ch === "\"") inString = true;
    else if (ch === "{") depth += 1;
    else if (ch === "}") {
      depth -= 1;
      if (depth === 0) return trimmed.slice(start, i + 1);
    }
  }
  return trimmed;
}

export function parseGoalDecision(raw: string): GoalDecision {
  let parsed: unknown;
  try {
    parsed = JSON.parse(extractGoalDecisionJson(raw));
  } catch {
    return { status: "blocked", reason: "Goal decision was not valid JSON.", next: "" };
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    return { status: "blocked", reason: "Goal decision JSON was not an object.", next: "" };
  }
  const record = parsed as Record<string, unknown>;
  const status = record.status;
  if (!DECISION_STATUSES.includes(status as GoalDecisionStatus)) {
    return { status: "blocked", reason: "Goal decision status was invalid.", next: "" };
  }
  return {
    status: status as GoalDecisionStatus,
    reason: stringValue(record.reason).trim() || "No reason provided.",
    next: stringValue(record.next),
  };
}

export function applyGoalDecision(goal: GoalSettings, decision: GoalDecision): GoalSettings {
  const turns = goal.turns + 1;
  if (decision.status === "complete") {
    return normalizeGoalSettings({
      ...goal,
      status: "complete",
      turns,
      lastReason: decision.reason,
      nextPrompt: "",
    });
  }
  if (decision.status === "blocked") {
    return normalizeGoalSettings({
      ...goal,
      status: "blocked",
      turns,
      lastReason: decision.reason,
      nextPrompt: decision.next,
    });
  }
  if (goal.developerMaxTurns && turns >= goal.developerMaxTurns) {
    return normalizeGoalSettings({
      ...goal,
      status: "paused",
      turns,
      lastReason: "Developer max-turn cap reached.",
      nextPrompt: decision.next,
    });
  }
  return normalizeGoalSettings({
    ...goal,
    status: "running",
    turns,
    lastReason: decision.reason,
    nextPrompt: decision.next,
  });
}
