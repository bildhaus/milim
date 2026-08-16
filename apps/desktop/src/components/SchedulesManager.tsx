import { useEffect, useMemo, useRef, useState } from "react";
import { useAgents } from "../agents/store";
import {
  MAX_ATTACHMENT_BYTES,
  createSchedule,
  deleteSchedule,
  inferAttachmentMime,
  isAccountRuntimeModel,
  listModels,
  listSchedules,
  pickAttachmentFiles,
  updateSchedule,
  type ChatAttachment,
  type ScheduleInfo,
} from "../api";
import { readBrowserAttachmentDataUrl } from "../lib/attachmentInput";
import { useSessions } from "../sessions/store";
import { Calendar, Paperclip, Plus, Trash, X } from "./icons";
import { AgentAvatar } from "./AgentAvatar";
import { SheetDialog } from "./SheetDialog";
import { Select, Toggle } from "./ui";
import "./SchedulesManager.css";

type Selection = ScheduleInfo | "new" | null;

type SchedulePreset = {
  key: string;
  label: string;
  name: string;
  cron: string;
  prompt: string;
  description: string;
};

type CronStatus = {
  valid: boolean;
  message: string;
  preview: string;
};

const DEFAULT_CRON = "0 0 9 * * Mon-Fri";
const MAX_SCHEDULE_ATTACHMENTS = 12;
const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const QUICK_CREATE_PRESETS: SchedulePreset[] = [
  {
    key: "weekday-digest",
    label: "Weekday digest",
    name: "Weekday digest",
    cron: DEFAULT_CRON,
    prompt: "Summarize the current project status and flag follow-ups.",
    description: "A calm weekday brief for projects, blockers, and next actions.",
  },
  {
    key: "daily",
    label: "Daily",
    name: "Daily check-in",
    cron: "0 0 9 * * *",
    prompt: "Review priorities and summarize what changed since yesterday.",
    description: "A morning scan for priorities and anything that shifted.",
  },
  {
    key: "hourly",
    label: "Hourly",
    name: "Hourly pulse",
    cron: "0 0 * * * *",
    prompt: "Check for important updates and summarize only actionable changes.",
    description: "A lightweight pulse for fast-moving work without long summaries.",
  },
  {
    key: "custom",
    label: "Custom",
    name: "",
    cron: DEFAULT_CRON,
    prompt: "",
    description: "Start blank with your own cadence, agent, and prompt.",
  },
];

const CRON_PRESETS = [
  { label: "Weekdays 9:00", cron: DEFAULT_CRON, preview: "Weekdays at 09:00" },
  { label: "Daily 9:00", cron: "0 0 9 * * *", preview: "Daily at 09:00" },
  { label: "Hourly", cron: "0 0 * * * *", preview: "Hourly" },
  { label: "Every 15 min", cron: "0 */15 * * * *", preview: "Every 15 minutes" },
];

const CRON_FIELD_NAMES = ["sec", "min", "hour", "day", "month", "dow"];
const CRON_FIELD_RANGES: Array<[number, number] | null> = [
  [0, 59],
  [0, 59],
  [0, 23],
  [1, 31],
  [1, 12],
  null,
];

function normalizeCron(cron: string): string {
  return cron.trim().replace(/\s+/g, " ");
}

function cronFields(cron: string): string[] {
  const normalized = normalizeCron(cron);
  return normalized ? normalized.split(" ") : [];
}

function agentLabel(agentId: string | null | undefined, agents: Array<{ id: string; name: string }>): string {
  if (!agentId) return "Default agent";
  return agents.find((a) => a.id === agentId)?.name ?? agentId;
}

function scheduleModel(schedule: ScheduleInfo): string {
  return ((schedule as ScheduleInfo & { model?: string }).model ?? "").trim();
}

function effectiveScheduleModel(schedule: ScheduleInfo, agents: Array<{ id: string; model: string }>): string {
  return scheduleModel(schedule) || agents.find((agent) => agent.id === schedule.agent_id)?.model?.trim() || "";
}

function plural(value: number, unit: string): string {
  return `${value} ${unit}${value === 1 ? "" : "s"}`;
}

function attachmentId(): string {
  return crypto.randomUUID();
}

function attachmentSizeLabel(size: number): string {
  if (size >= 1024 * 1024) return `${(size / 1024 / 1024).toFixed(1)} MB`;
  if (size >= 1024) return `${Math.round(size / 1024)} KB`;
  return `${size} B`;
}

function attachmentSummary(attachments: ChatAttachment[] | undefined): string {
  const count = attachments?.length ?? 0;
  return count ? plural(count, "attachment") : "No attachments";
}

function attachmentFingerprint(attachments: ChatAttachment[]): string {
  return JSON.stringify(
    attachments.map((attachment) => ({
      id: attachment.id,
      name: attachment.name,
      mime: attachment.mime,
      size: attachment.size,
      content: attachment.content,
      dataUrl: attachment.dataUrl
        ? `${attachment.dataUrl.length}:${attachment.dataUrl.slice(-32)}`
        : "",
      truncated: Boolean(attachment.truncated),
      sourcePath: attachment.sourcePath ?? "",
    })),
  );
}

function textLikeMime(mime: string): boolean {
  return (
    mime.startsWith("text/") ||
    mime === "application/json" ||
    mime === "application/xml" ||
    mime === "application/javascript"
  );
}

async function browserFileAttachment(file: File): Promise<ChatAttachment> {
  const mime = file.type || inferAttachmentMime(file.name);
  const [content, dataUrl] = await Promise.all([
    textLikeMime(mime)
      ? file.slice(0, MAX_ATTACHMENT_BYTES).text()
      : Promise.resolve(undefined),
    readBrowserAttachmentDataUrl(file, mime),
  ]);
  return {
    id: attachmentId(),
    name: file.name || "attachment",
    mime,
    size: file.size,
    content,
    dataUrl,
    truncated: textLikeMime(mime) ? file.size > MAX_ATTACHMENT_BYTES : false,
  };
}

function lastRunLabel(lastRun: number | null | undefined): string {
  if (!lastRun) return "Never run";
  const seconds = Math.max(0, Math.floor((Date.now() - lastRun * 1000) / 1000));
  if (seconds < 60) return "Last run just now";
  if (seconds < 3600) return `Last run ${plural(Math.floor(seconds / 60), "minute")} ago`;
  if (seconds < 86400) return `Last run ${plural(Math.floor(seconds / 3600), "hour")} ago`;
  if (seconds < 1209600) return `Last run ${plural(Math.floor(seconds / 86400), "day")} ago`;
  return `Last run ${new Date(lastRun * 1000).toLocaleDateString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  })}`;
}

function numericField(value: string, min: number, max: number): number | null {
  if (!/^\d+$/.test(value)) return null;
  const n = Number(value);
  return n >= min && n <= max ? n : null;
}

function intervalField(value: string): number | null {
  const match = value.match(/^\*\/(\d+)$/);
  if (!match) return null;
  const interval = Number(match[1]);
  return interval > 0 ? interval : null;
}

function timeLabel(hour: number, minute: number, second: number): string {
  const hh = String(hour).padStart(2, "0");
  const mm = String(minute).padStart(2, "0");
  const ss = String(second).padStart(2, "0");
  return second === 0 ? `${hh}:${mm}` : `${hh}:${mm}:${ss}`;
}

function weekdayLabel(dayOfWeek: string): string | null {
  const normalized = dayOfWeek.toLowerCase();
  const labels: Record<string, string> = {
    mon: "Monday",
    monday: "Monday",
    tue: "Tuesday",
    tuesday: "Tuesday",
    wed: "Wednesday",
    wednesday: "Wednesday",
    thu: "Thursday",
    thursday: "Thursday",
    fri: "Friday",
    friday: "Friday",
    sat: "Saturday",
    saturday: "Saturday",
    sun: "Sunday",
    sunday: "Sunday",
  };
  return labels[normalized] ?? null;
}

function isWeekdayRange(dayOfWeek: string): boolean {
  const normalized = dayOfWeek.toLowerCase();
  return normalized === "mon-fri" || normalized === "monday-friday" || normalized === "1-5";
}

function describeCron(cron: string): string {
  const normalized = normalizeCron(cron);
  const preset = CRON_PRESETS.find((p) => p.cron === normalized);
  if (preset) return preset.preview;

  const parts = cronFields(cron);
  if (parts.length !== 6) return "Invalid cadence";

  const [sec, min, hour, day, month, dow] = parts;
  const secN = numericField(sec, 0, 59);
  const minN = numericField(min, 0, 59);
  const hourN = numericField(hour, 0, 23);
  const dayN = numericField(day, 1, 31);
  const minInterval = intervalField(min);
  const secInterval = intervalField(sec);
  const runsOnAnyDate = day === "*" && month === "*";

  if (runsOnAnyDate && secN !== null && minN !== null && hourN !== null) {
    const time = timeLabel(hourN, minN, secN);
    if (dow === "*") return `Daily at ${time}`;
    if (isWeekdayRange(dow)) return `Weekdays at ${time}`;
    const weekday = weekdayLabel(dow);
    if (weekday) return `Every ${weekday} at ${time}`;
  }

  if (runsOnAnyDate && dow === "*" && hour === "*" && secN !== null && minN !== null) {
    return minN === 0 ? "Hourly" : `Hourly at :${String(minN).padStart(2, "0")}`;
  }

  if (runsOnAnyDate && dow === "*" && hour === "*" && secN !== null && minInterval !== null) {
    return `Every ${plural(minInterval, "minute")}`;
  }

  if (runsOnAnyDate && dow === "*" && hour === "*" && min === "*" && secInterval !== null) {
    return `Every ${plural(secInterval, "second")}`;
  }

  if (month === "*" && dow === "*" && dayN !== null && secN !== null && minN !== null && hourN !== null) {
    return `Monthly on day ${dayN} at ${timeLabel(hourN, minN, secN)}`;
  }

  return "Custom cadence";
}

function validateCron(cron: string): CronStatus {
  const normalized = normalizeCron(cron);
  const preview = describeCron(cron);
  if (!normalized) {
    return { valid: false, message: "Cron is required.", preview };
  }

  const parts = cronFields(cron);
  if (parts.length !== 6) {
    return {
      valid: false,
      message: `Use six fields: sec min hour day month dow. Found ${parts.length}.`,
      preview,
    };
  }

  const badField = parts.findIndex((field) => !/^[A-Za-z0-9*?,/\-#LW]+$/.test(field));
  if (badField >= 0) {
    return {
      valid: false,
      message: `${CRON_FIELD_NAMES[badField]} contains unsupported characters.`,
      preview,
    };
  }

  const outOfRange = parts.findIndex((field, index) => {
    const range = CRON_FIELD_RANGES[index];
    if (!range || !/^\d+$/.test(field)) return false;
    const value = Number(field);
    return value < range[0] || value > range[1];
  });
  if (outOfRange >= 0) {
    const range = CRON_FIELD_RANGES[outOfRange];
    return {
      valid: false,
      message: `${CRON_FIELD_NAMES[outOfRange]} must be ${range?.[0]}-${range?.[1]}.`,
      preview,
    };
  }

  return {
    valid: true,
    message: "Looks like a six-field cron expression.",
    preview,
  };
}

function ScheduleStarterGrid({ onPreset }: { onPreset: (preset: SchedulePreset) => void }) {
  return (
    <div className="schedule-starter-grid" aria-label="Schedule starters">
      {QUICK_CREATE_PRESETS.map((preset) => (
        <button
          key={preset.key}
          type="button"
          className="schedule-starter-card"
          onClick={() => onPreset(preset)}
          aria-label={`Create ${preset.label} schedule`}
        >
          <span className="schedule-starter-mark" aria-hidden="true">
            <Calendar size={16} />
          </span>
          <span className="schedule-starter-copy">
            <strong>{preset.label}</strong>
            <span>{preset.key === "custom" ? "Custom cadence" : describeCron(preset.cron)}</span>
            <small>{preset.description}</small>
          </span>
        </button>
      ))}
    </div>
  );
}

function ScheduleReadiness({
  tone,
  title,
  body,
  counts,
}: {
  tone: "ready" | "warning" | "off" | "draft";
  title: string;
  body: string;
  counts: string[];
}) {
  return (
    <div className={"schedule-readiness " + tone}>
      <div className="schedule-readiness-main">
        <span className={"schedule-readiness-dot " + tone} aria-hidden="true" />
        <div>
          <strong>{title}</strong>
          <span>{body}</span>
        </div>
      </div>
      <div className="schedule-readiness-counts" aria-label="Schedule summary">
        {counts.map((count) => (
          <span key={count}>{count}</span>
        ))}
      </div>
    </div>
  );
}

function ScheduleEmptyState({
  compact,
  title = "No schedules yet",
  body = "Choose a starter and tune the cadence, agent, and prompt before saving.",
  onPreset,
}: {
  compact?: boolean;
  title?: string;
  body?: string;
  onPreset: (preset: SchedulePreset) => void;
}) {
  return (
    <div className={"schedule-empty" + (compact ? " compact" : "")}>
      <div className="schedule-empty-copy">
        <div className="schedule-empty-icon" aria-hidden="true">
          <Calendar size={18} />
        </div>
        <div>
          <h3>{title}</h3>
          <p>{body}</p>
        </div>
      </div>
      <ScheduleStarterGrid onPreset={onPreset} />
    </div>
  );
}

function ScheduleListEmpty() {
  return (
    <div className="schedule-list-empty">
      <div>
        <Calendar size={15} />
        <strong>No schedules</strong>
      </div>
      <span>Pick a starter in the editor.</span>
    </div>
  );
}

function scheduleNoteTone(note: string): "error" | "warning" | "success" {
  if (note.startsWith("Error:")) return "error";
  if (note.startsWith("Click Delete again")) return "warning";
  return "success";
}

export function SchedulesManager({ onClose }: { onClose: () => void }) {
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const agents = useAgents((s) => s.agents);
  const refreshAgents = useAgents((s) => s.refresh);
  const activeSession = useSessions((s) => s.sessions.find((session) => session.id === s.activeId));
  const activeThreadModel = activeSession?.settings?.model?.trim() ?? "";
  const currentThreadModel = isAccountRuntimeModel(activeThreadModel)
    ? ""
    : activeThreadModel;

  const [schedules, setSchedules] = useState<ScheduleInfo[]>([]);
  const [models, setModels] = useState<string[]>([]);
  const [sel, setSel] = useState<Selection>(null);
  const [name, setName] = useState("");
  const [cron, setCron] = useState("");
  const [agentId, setAgentId] = useState("");
  const [model, setModel] = useState("");
  const [prompt, setPrompt] = useState("");
  const [attachments, setAttachments] = useState<ChatAttachment[]>([]);
  const [enabled, setEnabled] = useState(true);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  useEffect(() => {
    void listSchedules().then(setSchedules);
    void listModels().then((items) => setModels(items.filter((item) => !isAccountRuntimeModel(item))));
    void refreshAgents();
  }, [refreshAgents]);

  useEffect(() => {
    if (!sel || sel === "new" || model || scheduleModel(sel)) return;
    const fallback = agents.find((agent) => agent.id === sel.agent_id)?.model?.trim();
    if (fallback && !isAccountRuntimeModel(fallback)) setModel(fallback);
  }, [agents, model, sel]);

  async function refresh() {
    setSchedules(await listSchedules());
  }

  function edit(s: ScheduleInfo | "new") {
    setSel(s);
    setNote(null);
    setConfirmDeleteId(null);
    if (s === "new") {
      setName("");
      setCron(DEFAULT_CRON);
      setAgentId("");
      setModel(currentThreadModel);
      setPrompt("");
      setAttachments([]);
      setEnabled(true);
    } else {
      setName(s.name);
      setCron(s.cron);
      setAgentId(s.agent_id ?? "");
      const selectedModel = effectiveScheduleModel(s, agents);
      setModel(isAccountRuntimeModel(selectedModel) ? "" : selectedModel);
      setPrompt(s.prompt);
      setAttachments(s.attachments ?? []);
      setEnabled(s.enabled);
    }
  }

  function editPreset(preset: SchedulePreset) {
    setSel("new");
    setNote(null);
    setConfirmDeleteId(null);
    setName(preset.name);
    setCron(preset.cron);
    setAgentId("");
    setModel(currentThreadModel);
    setPrompt(preset.prompt);
    setAttachments([]);
    setEnabled(true);
  }

  async function attachFiles(files?: File[]) {
    try {
      let next: ChatAttachment[] = [];
      if (files?.length) {
        next = await Promise.all(files.map(browserFileAttachment));
      } else if (isTauri) {
        next = (await pickAttachmentFiles()).map((attachment) => ({
          id: attachmentId(),
          ...attachment,
        }));
      } else {
        fileInputRef.current?.click();
        return;
      }
      if (next.length) {
        setAttachments((current) => [...current, ...next].slice(0, MAX_SCHEDULE_ATTACHMENTS));
        setNote(null);
      }
    } catch (e) {
      setNote(`Error: Could not attach file: ${e instanceof Error ? e.message : String(e)}`);
    }
  }

  function removeAttachment(id: string) {
    setAttachments((current) => current.filter((attachment) => attachment.id !== id));
  }

  async function save() {
    const cronStatus = validateCron(cron);
    if (!name.trim() || !cron.trim() || !model.trim() || !prompt.trim() || !cronStatus.valid) return;
    setBusy(true);
    setNote(null);
    setConfirmDeleteId(null);
    const agent_id = agentId || null;
    const payload = {
      name: name.trim(),
      cron: normalizeCron(cron),
      agent_id,
      model: model.trim(),
      prompt: prompt.trim(),
      attachments,
    };
    const saved =
      sel && sel !== "new"
        ? await updateSchedule({
            ...payload,
            id: sel.id,
            enabled,
            last_run: sel.last_run ?? null,
          })
        : await createSchedule(payload);

    const finalSchedule = saved && saved.enabled !== enabled ? await updateSchedule({ ...saved, enabled }) : saved;
    setBusy(false);
    if (!finalSchedule) {
      setNote("Error: Failed to save schedule. Check the cron expression.");
      return;
    }
    await refresh();
    setSel(finalSchedule);
    setNote(finalSchedule.enabled ? "Schedule saved." : "Schedule saved disabled.");
  }

  async function remove() {
    if (!sel || sel === "new") return;
    if (confirmDeleteId !== sel.id) {
      setConfirmDeleteId(sel.id);
      setNote(`Click Delete again to remove "${sel.name}".`);
      return;
    }
    await deleteSchedule(sel.id);
    await refresh();
    setConfirmDeleteId(null);
    setSel(null);
    setNote(null);
  }

  const agentOptions = useMemo(
    () => [
      { label: "Default agent", value: "" },
      ...agents.map((agent) => ({
        label: agent.name,
        value: agent.id,
        leading: <AgentAvatar id={agent.id} name={agent.name} avatar={agent.avatar} className="select-agent-avatar" />,
      })),
    ],
    [agents],
  );
  const modelOptions = useMemo(
    () =>
      Array.from(new Set([model, currentThreadModel, ...models].map((value) => value.trim()).filter((value) => value && !isAccountRuntimeModel(value)))).map(
        (value) => ({ label: value, value }),
      ),
    [currentThreadModel, model, models],
  );
  const cronStatus = useMemo(() => validateCron(cron), [cron]);
  const activeCount = schedules.filter((s) => s.enabled).length;
  const selectedSchedule = sel && sel !== "new" ? sel : null;
  const currentAttachmentFingerprint = useMemo(() => attachmentFingerprint(attachments), [attachments]);
  const selectedAttachmentFingerprint = useMemo(
    () => attachmentFingerprint(selectedSchedule?.attachments ?? []),
    [selectedSchedule],
  );
  const hasDraftContent = Boolean(
    name.trim() || prompt.trim() || attachments.length > 0 || agentId || !enabled || normalizeCron(cron) !== DEFAULT_CRON,
  );
  const formComplete = Boolean(name.trim() && cron.trim() && model.trim() && prompt.trim());
  const formReady = formComplete && cronStatus.valid;
  const isDirty =
    sel === "new"
      ? hasDraftContent
      : Boolean(
          selectedSchedule &&
            (name !== selectedSchedule.name ||
              normalizeCron(cron) !== normalizeCron(selectedSchedule.cron) ||
              (agentId || "") !== (selectedSchedule.agent_id ?? "") ||
              model.trim() !== scheduleModel(selectedSchedule) ||
              prompt !== selectedSchedule.prompt ||
              currentAttachmentFingerprint !== selectedAttachmentFingerprint ||
              enabled !== selectedSchedule.enabled),
        );
  const canSave = Boolean(formReady && !busy && (sel === "new" || isDirty));
  const editorTitle = sel === "new" ? "New schedule" : sel ? name || sel.name : "Select a schedule";
  const editorTone: "ready" | "warning" | "off" | "draft" =
    !formComplete || !cronStatus.valid ? "warning" : enabled ? "ready" : "off";
  const editorReadinessTitle =
    !model.trim() ? "Needs model" : !formComplete ? "Needs details" : !cronStatus.valid ? "Fix cadence" : enabled ? "Ready to run" : "Paused";
  const editorReadinessBody = !model.trim()
    ? "Choose the model this unattended run should use."
    : !formComplete
      ? "Add a name, cadence, and prompt before saving."
    : !cronStatus.valid
      ? cronStatus.message
      : enabled
        ? `${cronStatus.preview} with ${agentLabel(agentId, agents)}.`
        : "This automation is saved but will not run until enabled.";
  const saveLabel = busy ? "Saving..." : sel === "new" ? "Create schedule" : isDirty ? "Save changes" : "Saved";

  return (
    <SheetDialog title="Schedules" className="sheet agents-sheet schedule-sheet" onClose={onClose}>
        <input
          ref={fileInputRef}
          type="file"
          multiple
          hidden
          onChange={(e) => {
            const files = Array.from(e.currentTarget.files ?? []);
            e.currentTarget.value = "";
            if (files.length) void attachFiles(files);
          }}
        />
        <div className="schedule-header">
          <div className="schedule-title">
            <h2>Schedules</h2>
            <p>Run a saved agent prompt on a cron schedule. Cron uses six fields: sec min hour day month dow.</p>
            <div className="schedule-header-meta" aria-label="Schedule counts">
              <span>{schedules.length} total</span>
              <span>{activeCount} active</span>
            </div>
          </div>
          <div className="schedule-header-actions">
            <button className="btn-accent schedule-new-action" type="button" onClick={() => edit("new")}>
              <Plus size={14} />
              <span>New schedule</span>
            </button>
            <button className="icon-btn sheet-close schedule-close" type="button" onClick={onClose} title="Close" aria-label="Close schedules">
              <X size={15} />
            </button>
          </div>
        </div>

        <div className="schedule-manager">
          <aside className="schedule-list-panel" aria-label="Schedule list">
            <div className="schedule-list-summary">
              <span>{schedules.length} total</span>
              <span>{activeCount} active</span>
            </div>
            <button className="schedule-rail-action" type="button" onClick={() => edit("new")}>
              <Plus size={14} />
              <span>New</span>
            </button>

            {schedules.length > 0 ? (
              <div className="schedule-list">
                {schedules.map((s) => {
                  const active = selectedSchedule?.id === s.id;
                  const scheduleAgent = agents.find((agent) => agent.id === s.agent_id);
                  return (
                    <button
                      key={s.id}
                      type="button"
                      className={"schedule-row" + (active ? " active" : "")}
                      onClick={() => edit(s)}
                    >
                      <span className={"schedule-row-status " + (s.enabled ? "enabled" : "disabled")} aria-hidden="true" />
                      <span className="schedule-row-copy">
                        <span className="schedule-row-top">
                          <span className="schedule-row-name">{s.name}</span>
                          <span className={"schedule-row-state " + (s.enabled ? "enabled" : "disabled")}>{s.enabled ? "On" : "Off"}</span>
                        </span>
                        <span className="schedule-row-cadence">{describeCron(s.cron)}</span>
                        <span className="schedule-row-cron">{s.cron}</span>
                        <span className="schedule-row-meta">
                          <span className="schedule-row-agent">
                            {scheduleAgent && <AgentAvatar id={scheduleAgent.id} name={scheduleAgent.name} avatar={scheduleAgent.avatar} className="schedule-agent-avatar" />}
                            <span>{agentLabel(s.agent_id, agents)}</span>
                          </span>
                          <span>{effectiveScheduleModel(s, agents) || "Model missing"}</span>
                          <span>{attachmentSummary(s.attachments)}</span>
                          <span>{lastRunLabel(s.last_run)}</span>
                        </span>
                      </span>
                    </button>
                  );
                })}
              </div>
            ) : (
              <ScheduleListEmpty />
            )}
          </aside>

          <section className="schedule-editor-panel" aria-label="Schedule editor">
            {sel ? (
              <div className="schedule-editor">
                <ScheduleReadiness
                  tone={editorTone}
                  title={editorReadinessTitle}
                  body={editorReadinessBody}
                  counts={[sel === "new" ? "Draft" : isDirty ? "Unsaved" : "Saved", enabled ? "Enabled" : "Paused"]}
                />

                <div className="schedule-editor-head">
                  <div>
                    <span className="schedule-kicker">{sel === "new" ? "Draft automation" : enabled ? "Active automation" : "Paused automation"}</span>
                    <h3>{editorTitle}</h3>
                  </div>
                  <span className={"schedule-editor-status " + (enabled ? "enabled" : "disabled")}>{enabled ? "Enabled" : "Disabled"}</span>
                </div>

                <section className="schedule-editor-section">
                  <div className="schedule-section-head">
                    <h4>Trigger</h4>
                    <span>{cronStatus.preview}</span>
                  </div>
                  <div className={"schedule-cadence-card " + (cronStatus.valid ? "valid" : "invalid")}>
                    <span className="schedule-cadence-icon" aria-hidden="true">
                      <Calendar size={16} />
                    </span>
                    <div>
                      <strong>{cronStatus.preview}</strong>
                      <span>{cronStatus.valid ? "Six-field cron is ready." : cronStatus.message}</span>
                    </div>
                  </div>
                  <label className="schedule-field">
                    <span>Name</span>
                    <input className="css-input schedule-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Weekday digest" />
                  </label>
                  <div className="schedule-preset-field">
                    <span>Cadence presets</span>
                    <div className="schedule-cron-presets" aria-label="Cron presets">
                      {CRON_PRESETS.map((preset) => (
                        <button
                          key={preset.cron}
                          type="button"
                          className={"schedule-preset-btn" + (normalizeCron(cron) === preset.cron ? " active" : "")}
                          onClick={() => setCron(preset.cron)}
                        >
                          {preset.label}
                        </button>
                      ))}
                    </div>
                  </div>
                  <label className="schedule-field">
                    <span>Cron expression</span>
                    <input
                      className={"css-input schedule-input schedule-cron-input" + (cronStatus.valid ? "" : " invalid")}
                      value={cron}
                      onChange={(e) => setCron(e.target.value)}
                      placeholder={DEFAULT_CRON}
                    />
                  </label>
                  <p className={"schedule-cron-status" + (cronStatus.valid ? " valid" : " invalid")}>
                    {cronStatus.message} Preview: {cronStatus.preview}.
                  </p>
                </section>

                <section className="schedule-editor-section">
                  <div className="schedule-section-head">
                    <h4>Execution</h4>
                    <span>{model || "Model required"}</span>
                  </div>
                  <div className="schedule-field">
                    <span>Model</span>
                    <Select value={model} placeholder="Choose model" options={modelOptions} onChange={setModel} testId="schedule-model-select" />
                  </div>
                  <div className="schedule-field">
                    <span>Agent</span>
                    <Select value={agentId} placeholder="Default agent" options={agentOptions} onChange={setAgentId} testId="schedule-agent-select" />
                  </div>
                </section>

                <section className="schedule-editor-section schedule-prompt-section">
                  <div className="schedule-section-head">
                    <h4>Prompt</h4>
                    <span>
                      {prompt.trim().length} chars, {attachmentSummary(attachments)}
                    </span>
                  </div>
                  <textarea
                    className="instr-input schedule-prompt"
                    value={prompt}
                    onChange={(e) => setPrompt(e.target.value)}
                    placeholder="Summarize the current project status."
                  />
                </section>

                <section className="schedule-editor-section">
                  <div className="schedule-section-head">
                    <h4>Attachments</h4>
                    <span>
                      {attachments.length}/{MAX_SCHEDULE_ATTACHMENTS}
                    </span>
                  </div>
                  <div className="schedule-attachment-card">
                    <button
                      className="schedule-attachment-add"
                      type="button"
                      disabled={busy || attachments.length >= MAX_SCHEDULE_ATTACHMENTS}
                      onClick={() => void attachFiles()}
                    >
                      <Paperclip size={14} />
                      <span>Attach files</span>
                    </button>
                    {attachments.length > 0 ? (
                      <div className="schedule-attachment-list">
                        {attachments.map((attachment) => (
                          <span key={attachment.id} className="schedule-attachment-pill" title={attachment.name}>
                            <Paperclip size={13} />
                            <span className="schedule-attachment-name">{attachment.name}</span>
                            <span className="schedule-attachment-meta">
                              {attachmentSizeLabel(attachment.size)}
                              {attachment.truncated ? " clipped" : ""}
                            </span>
                            <button
                              type="button"
                              className="schedule-attachment-remove"
                              title="Remove attachment"
                              aria-label={`Remove attachment ${attachment.name}`}
                              onClick={() => removeAttachment(attachment.id)}
                            >
                              <X size={12} />
                            </button>
                          </span>
                        ))}
                      </div>
                    ) : (
                      <span className="schedule-attachment-empty">No attached files</span>
                    )}
                  </div>
                </section>

                <section className="schedule-editor-section">
                  <div className="schedule-section-head">
                    <h4>Status</h4>
                    <span>{sel === "new" ? "Not saved yet" : lastRunLabel(sel.last_run)}</span>
                  </div>
                  <div className="schedule-status-card">
                    <Toggle checked={enabled} onChange={setEnabled} label="Enabled" />
                    <div>
                      <strong>{enabled ? "Runs automatically" : "Paused"}</strong>
                      <span>{enabled ? "This schedule will run when its cron is due." : "This schedule is saved but will not run."}</span>
                    </div>
                  </div>
                  {note && <p className={"schedule-note " + scheduleNoteTone(note)}>{note}</p>}
                </section>

                <div className="schedule-action-footer">
                  {sel !== "new" && (
                    <button className="btn-ghost danger schedule-delete-action" type="button" disabled={busy} onClick={remove}>
                      <Trash size={14} />
                      <span>{confirmDeleteId === sel.id ? "Confirm delete" : "Delete"}</span>
                    </button>
                  )}
                  <span className="schedule-footer-spacer" />
                  <button className="btn-accent" type="button" disabled={!canSave} onClick={save}>
                    {saveLabel}
                  </button>
                </div>
              </div>
            ) : (
              <div className="schedule-overview">
                <ScheduleReadiness
                  tone={schedules.length > 0 ? "draft" : "off"}
                  title={schedules.length > 0 ? "Choose an automation" : "Build your first automation"}
                  body={
                    schedules.length > 0
                      ? "Select a saved schedule from the rail or start another from a recipe."
                      : "Start from a practical cadence, then adjust the details before saving."
                  }
                  counts={[`${schedules.length} total`, `${activeCount} active`]}
                />
                <ScheduleEmptyState
                  title={schedules.length > 0 ? "Select a schedule" : undefined}
                  body={
                    schedules.length > 0
                      ? "Choose an automation from the list to edit it, or start another schedule from a preset."
                      : undefined
                  }
                  onPreset={editPreset}
                />
              </div>
            )}
          </section>
        </div>
      </SheetDialog>
  );
}
