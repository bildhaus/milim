import { type KeyboardEvent, useEffect, useMemo, useRef, useState } from "react";
import { type Agent, type ChatAttachment, type MediaKind, type SkillInfo, type ToolInfo } from "../api";
import type { WorkspaceFileSuggestion } from "../api";
import { composerAutocompleteTriggerAt, composerCommandRunsOnSelection, composerSuggestionMatchScore, mcpToolTagCompletion, replaceComposerAutocompleteTrigger, skillTagCompletion } from "../lib/composerAutocomplete";
import { canNavigateComposerHistory, moveComposerHistory, type ComposerHistoryDirection } from "../lib/composerHistory";
import { composerTokenParts, composerTokensForText } from "../lib/composerTokens";
import { shortcutLabel, shortcutMatchesEvent } from "../ui/shortcuts";
import { useUiPreferences } from "../ui/store";
import { AgentAvatar } from "./AgentAvatar";
import { ArrowUp, ChevronDown, Folder, FolderOpen, Paperclip, PlusSquare, Slash, Square, UserRound, X } from "./icons";
const COMPOSER_HISTORY_NOTICE_MS = 1800;

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

type SlashCommand = {
  id: string;
  label: string;
  hint: string;
  group: "Commands" | "Settings";
  placeholder?: string;
};

const SLASH_COMMANDS: SlashCommand[] = [
  { id: "plan", label: "Plan mode", hint: "Toggle read-only planning", group: "Commands", placeholder: "/plan build feature X" },
  { id: "goal", label: "Goal", hint: "Make the next prompt an autonomous goal", group: "Commands", placeholder: "/goal build feature X" },
  { id: "model", label: "Model", hint: "Set the thread model", group: "Settings", placeholder: "/model llama3.2" },
  { id: "folder", label: "Folder", hint: "Set or pick a working folder", group: "Settings", placeholder: "/folder C:\\project" },
  { id: "sandbox", label: "Sandbox on", hint: "Enable Docker sandbox tools", group: "Settings" },
  { id: "nosandbox", label: "Sandbox off", hint: "Disable Docker sandbox tools", group: "Settings" },
  { id: "computer", label: "Computer on", hint: "Enable computer-use tools", group: "Settings" },
  { id: "nocomputer", label: "Computer off", hint: "Disable computer-use tools", group: "Settings" },
  { id: "memory", label: "Memory on", hint: "Enable scoped memories", group: "Settings" },
  { id: "nomemory", label: "Memory off", hint: "Disable scoped memories", group: "Settings" },
  { id: "privacy", label: "Privacy", hint: "Set privacy gate", group: "Settings", placeholder: "/privacy redact" },
  { id: "approval", label: "Approval", hint: "Set tool approval mode", group: "Settings", placeholder: "/approval guarded" },
  { id: "agent", label: "Agent", hint: "Set active agent or none", group: "Settings", placeholder: "/agent none" },
  { id: "compact", label: "Compact thread", hint: "Summarize prior context into a fresh checkpoint", group: "Commands" },
  { id: "export", label: "Export chat", hint: "Download this thread as JSON", group: "Commands" },
  { id: "import", label: "Import chat", hint: "Import a Milim thread JSON file", group: "Commands" },
  { id: "clear", label: "New chat", hint: "Start a fresh chat with current settings", group: "Commands" },
];

function agentMenuDetail(agent: Agent): string {
  const mode = agent.tool_mode ?? ((agent.enabled_tools ?? []).length === 0 ? "all" : "custom");
  const tools = mode === "all" ? "all tools" : mode === "none" ? "no tools" : `${agent.enabled_tools?.length ?? 0} tools`;
  const skillMode = agent.skill_mode ?? ((agent.enabled_skills ?? []).length === 0 ? "auto" : "custom");
  const skills = skillMode === "auto" ? "auto skills" : skillMode === "none" ? "no skills" : `${agent.enabled_skills?.length ?? 0} skills`;
  return `${tools} / ${skills}`;
}

type Suggestion =
  | { kind: "action"; group: "Add"; key: string; name: string; label: string; hint: string }
  | { kind: "command"; group: SlashCommand["group"]; command: SlashCommand }
  | { kind: "file"; group: "Files"; file: WorkspaceFileSuggestion }
  | { kind: "mcp"; group: "MCP"; tool: ToolInfo }
  | { kind: "skill"; group: "Skills"; skill: SkillInfo };

type WorkspaceProject = {
  name: string;
  folder: string;
};

function parseSlashInput(value: string): { id: string; argument: string } | null {
  const match = value.trim().match(/^\/([a-z-]+)(?:\s+(.*))?$/i);
  if (!match) return null;
  return { id: match[1].toLowerCase(), argument: match[2]?.trim() ?? "" };
}

function attachmentSizeLabel(size: number): string {
  if (size >= 1024 * 1024) return `${(size / 1024 / 1024).toFixed(1)} MB`;
  if (size >= 1024) return `${Math.round(size / 1024)} KB`;
  return `${size} B`;
}

function folderLabel(folder: string): string {
  return folder.split(/[\\/]/).filter(Boolean).pop() || folder || "No project";
}

function clipboardFiles(data: DataTransfer): File[] {
  const byKey = new Map<string, File>();
  for (const file of Array.from(data.files ?? [])) {
    byKey.set(`${file.name}:${file.size}:${file.type}:${file.lastModified}`, file);
  }
  for (const item of Array.from(data.items ?? [])) {
    if (item.kind !== "file") continue;
    const file = item.getAsFile();
    if (file) byKey.set(`${file.name}:${file.size}:${file.type}:${file.lastModified}`, file);
  }
  return Array.from(byKey.values());
}

export function Composer({
  value,
  onChange,
  onSend,
  onStop,
  attachments,
  onAttachFiles,
  onAttachWorkspaceFile,
  onRemoveAttachment,
  onSlashCommand,
  agents,
  activeAgentId,
  onAgent,
  onManageAgents,
  instructions,
  onInstructions,
  skills = [],
  tools = [],
  workspaceFolder = "",
  workspaceProjects = [],
  onWorkspaceFolder,
  onPickWorkspaceFolder,
  listWorkspaceFiles,
  mediaActive = false,
  mediaKind = "image",
  mediaTargetLabel,
  sentHistory = [],
  requestCompletion,
  tokens,
  contextBudgetTokens,
  busy,
}: {
  value: string;
  onChange: (v: string) => void;
  onSend: () => void;
  onStop: () => void;
  attachments: ChatAttachment[];
  onAttachFiles: (files?: File[]) => void;
  onAttachWorkspaceFile: (file: WorkspaceFileSuggestion) => Promise<boolean>;
  onRemoveAttachment: (id: string) => void;
  onSlashCommand: (id: string, argument: string) => boolean;
  agents: Agent[];
  activeAgentId: string | null;
  onAgent: (agent: Agent | null) => void;
  onManageAgents: () => void;
  instructions: string;
  onInstructions: (v: string) => void;
  skills?: SkillInfo[];
  tools?: ToolInfo[];
  workspaceFolder?: string;
  workspaceProjects?: WorkspaceProject[];
  onWorkspaceFolder: (folder: string) => void;
  onPickWorkspaceFolder: () => void;
  listWorkspaceFiles: (workspace: string, query: string, limit?: number) => Promise<WorkspaceFileSuggestion[]>;
  mediaActive?: boolean;
  mediaKind?: MediaKind;
  mediaTargetLabel?: string;
  sentHistory?: string[];
  requestCompletion?: (text: string, signal: AbortSignal) => Promise<string | null>;
  tokens: number;
  contextBudgetTokens?: number;
  busy: boolean;
}) {
  const ref = useRef<HTMLTextAreaElement>(null);
  const highlightRef = useRef<HTMLDivElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const personaRef = useRef<HTMLDivElement>(null);
  const projectRef = useRef<HTMLDivElement>(null);
  const composerSendShortcut = useUiPreferences((s) => s.composerSendShortcut);
  const composerDensity = useUiPreferences((s) => s.composerDensity);
  const autocompleteMode = useUiPreferences((s) => s.autocompleteMode);
  const autocompleteSources = useUiPreferences((s) => s.autocompleteSources);
  const personalizedSuggestions = useUiPreferences((s) => s.personalizedSuggestions);
  const suggestionUsage = useUiPreferences((s) => s.suggestionUsage);
  const recordSuggestionUse = useUiPreferences((s) => s.recordSuggestionUse);
  const valueRef = useRef(value);
  const applyingHistoryRef = useRef(false);
  const historyDraftRef = useRef("");
  const historyNoticeTimerRef = useRef<number | null>(null);
  const completionControllerRef = useRef<AbortController | null>(null);
  const [personaOpen, setPersonaOpen] = useState(false);
  const [projectOpen, setProjectOpen] = useState(false);
  const [slashOpen, setSlashOpen] = useState(false);
  const [slashFocusIndex, setSlashFocusIndex] = useState(0);
  const [slashDismissedValue, setSlashDismissedValue] = useState("");
  const [cursor, setCursor] = useState(0);
  const [workspaceFiles, setWorkspaceFiles] = useState<WorkspaceFileSuggestion[]>([]);
  const [dragOver, setDragOver] = useState(false);
  const [historyIndex, setHistoryIndex] = useState<number | null>(null);
  const [historyNotice, setHistoryNotice] = useState<string | null>(null);
  const [ghostCompletion, setGhostCompletion] = useState("");
  const [imeComposing, setImeComposing] = useState(false);

  const slashInput = parseSlashInput(value);
  const activeTrigger = composerAutocompleteTriggerAt(value, cursor);
  const suggestionPrefix = activeTrigger?.prefix ?? "";
  const suggestionQuery = activeTrigger?.query ?? "";
  const tokenLabel = tokens >= 1000 ? `~${(tokens / 1000).toFixed(1)}k` : `~${tokens}`;
  const contextTokenLabel = contextBudgetTokens
    ? `${tokenLabel} / ${contextBudgetTokens >= 1000 ? `${(contextBudgetTokens / 1000).toFixed(contextBudgetTokens < 10_000 ? 1 : 0)}k` : contextBudgetTokens} tokens`
    : `${tokenLabel} tokens`;
  const activeAgent = agents.find((agent) => agent.id === activeAgentId) ?? null;
  const hasInstr = instructions.trim().length > 0;
  const activeWorkspaceFolder = workspaceFolder.trim();
  const activeWorkspaceLabel = activeWorkspaceFolder
    ? workspaceProjects.find((project) => project.folder === activeWorkspaceFolder)?.name ?? folderLabel(activeWorkspaceFolder)
    : "No project";
  const showWorkspaceSelector = true;
  const availableSlashCommands = autocompleteSources.commands ? SLASH_COMMANDS : [];
  const suggestions = useMemo(() => {
    const scored: Array<{ item: Suggestion; score: number; usageId?: string }> = [];
    const add = (item: Suggestion, text: string, usageId?: string) => {
      const matchScore = composerSuggestionMatchScore(text, suggestionQuery);
      if (matchScore == null) return;
      const usage = personalizedSuggestions && usageId ? suggestionUsage[usageId] : undefined;
      scored.push({ item, usageId, score: matchScore - Math.min(0.75, (usage?.count ?? 0) * 0.05) });
    };
    if (!suggestionPrefix && composerSuggestionMatchScore("files folders attach", suggestionQuery) != null) {
      add({ kind: "action", group: "Add", key: "files", name: "Files", label: "Files and folders", hint: "Attach local context" }, "files folders attach");
    }
    if (suggestionPrefix === "@" && autocompleteSources.files) {
      workspaceFiles.forEach((file) => add({ kind: "file", group: "Files", file }, `${file.name} ${file.path}`));
    }
    if (suggestionPrefix !== "@") {
      availableSlashCommands.forEach((command) => add(
        { kind: "command", group: command.group, command },
        `${command.id} ${command.label} ${command.hint}`,
        `command:${command.id}`,
      ));
    }
    if (suggestionPrefix === "/" && autocompleteSources.mcp) {
      tools.filter((tool) => tool.name.includes("__")).forEach((tool) => add(
        { kind: "mcp", group: "MCP", tool },
        `${tool.name} ${tool.description}`,
        `mcp:${tool.name}`,
      ));
    }
    if (autocompleteSources.skills) {
      skills.filter((skill) => skill.enabled).forEach((skill) => add(
        { kind: "skill", group: "Skills", skill },
        `${skill.name} ${skill.description}`,
        `skill:${skill.id}`,
      ));
    }
    return scored.sort((left, right) => left.score - right.score).map(({ item }) => item);
  }, [autocompleteSources, availableSlashCommands, personalizedSuggestions, skills, suggestionPrefix, suggestionQuery, suggestionUsage, tools, workspaceFiles]);
  const suggestionGroups = useMemo(() => {
    const groups: Array<{ label: Suggestion["group"]; items: Suggestion[] }> = [];
    for (const item of suggestions) {
      const group = groups.find((g) => g.label === item.group);
      if (group) group.items.push(item);
      else groups.push({ label: item.group, items: [item] });
    }
    return groups;
  }, [suggestions]);
  const orderedSuggestions = useMemo(() => suggestionGroups.flatMap((group) => group.items), [suggestionGroups]);
  const autoSlashMenuOpen = autocompleteMode === "automatic" && Boolean(activeTrigger) && orderedSuggestions.length > 0 && value !== slashDismissedValue;
  const showSlashMenu = autocompleteMode !== "off" && (slashOpen || autoSlashMenuOpen);
  const canSend = Boolean(value.trim() || attachments.length);
  const sendShortcutLabel = composerSendShortcut === "modEnter" ? shortcutLabel("Mod+Enter") : "Enter";
  const sentHistoryKey = useMemo(() => sentHistory.join("\0"), [sentHistory]);
  const placeholder = mediaActive
    ? `Describe the ${mediaKind} to generate...`
    : "Message or attach files...";
  const composerTokens = useMemo(
    () => composerTokensForText(value, { skills, tools, workspaceFiles }),
    [skills, tools, value, workspaceFiles],
  );
  const composerHighlightParts = useMemo(() => composerTokenParts(value, composerTokens), [composerTokens, value]);
  const hasTokenLayer = composerTokens.length > 0;
  const showComposerOverlay = hasTokenLayer || Boolean(ghostCompletion);

  // Auto-grow the textarea up to a cap.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "0px";
    const nextHeight = Math.min(el.scrollHeight, 160) + "px";
    el.style.height = nextHeight;
    if (highlightRef.current) {
      highlightRef.current.style.height = nextHeight;
      highlightRef.current.scrollTop = el.scrollTop;
      highlightRef.current.scrollLeft = el.scrollLeft;
    }
  }, [value]);

  useEffect(() => {
    valueRef.current = value;
  }, [value]);

  useEffect(() => {
    if (applyingHistoryRef.current) {
      applyingHistoryRef.current = false;
      return;
    }
    setHistoryIndex(null);
    hideHistoryNotice();
  }, [value]);

  useEffect(() => {
    historyDraftRef.current = "";
    setHistoryIndex(null);
    hideHistoryNotice();
  }, [sentHistoryKey]);

  useEffect(() => () => clearHistoryNoticeTimer(), []);

  useEffect(() => {
    completionControllerRef.current?.abort();
    completionControllerRef.current = null;
    setGhostCompletion("");
    if (!requestCompletion || busy || mediaActive || showSlashMenu || imeComposing || value.replace(/\s/g, "").length < 20) return;
    const controller = new AbortController();
    completionControllerRef.current = controller;
    const timer = window.setTimeout(() => {
      void requestCompletion(value, controller.signal)
        .then((completion) => {
          if (!controller.signal.aborted && completion) setGhostCompletion(completion);
        })
        .catch((error) => {
          if (!(error instanceof DOMException && error.name === "AbortError")) setGhostCompletion("");
        });
    }, 600);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [busy, imeComposing, mediaActive, requestCompletion, showSlashMenu, value]);

  useEffect(() => {
    const openSuggestions = () => {
      if (autocompleteMode === "off") return;
      ref.current?.focus();
      setSlashOpen(true);
    };
    window.addEventListener("milim:open-composer-suggestions", openSuggestions);
    return () => window.removeEventListener("milim:open-composer-suggestions", openSuggestions);
  }, [autocompleteMode]);

  useEffect(() => {
    setSlashFocusIndex(0);
  }, [suggestionPrefix, suggestionQuery, orderedSuggestions.length]);

  useEffect(() => {
    if (cursor > value.length) setCursor(value.length);
  }, [cursor, value.length]);

  useEffect(() => {
    if (autocompleteMode === "off" || !autocompleteSources.files || suggestionPrefix !== "@" || !workspaceFolder.trim()) {
      setWorkspaceFiles([]);
      return;
    }
    let cancelled = false;
    void listWorkspaceFiles(workspaceFolder, suggestionQuery, 12)
      .then((files) => {
        if (!cancelled) setWorkspaceFiles(files);
      })
      .catch(() => {
        if (!cancelled) setWorkspaceFiles([]);
      });
    return () => {
      cancelled = true;
    };
  }, [autocompleteMode, autocompleteSources.files, listWorkspaceFiles, suggestionPrefix, suggestionQuery, workspaceFolder]);

  useEffect(() => {
    if (!personaOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (personaRef.current && !personaRef.current.contains(e.target as Node)) setPersonaOpen(false);
    };
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") setPersonaOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [personaOpen]);

  useEffect(() => {
    if (!projectOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (projectRef.current && !projectRef.current.contains(e.target as Node)) setProjectOpen(false);
    };
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") setProjectOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [projectOpen]);

  function attachFromPicker() {
    if (isTauriRuntime()) {
      onAttachFiles();
      return;
    }
    fileRef.current?.click();
  }

  function syncCursor(target: HTMLTextAreaElement) {
    setCursor(target.selectionStart);
  }

  function syncHighlightScroll(target: HTMLTextAreaElement) {
    if (!highlightRef.current) return;
    highlightRef.current.scrollTop = target.scrollTop;
    highlightRef.current.scrollLeft = target.scrollLeft;
  }

  function runSlash(id: string, argument = "") {
    const handled = onSlashCommand(id, argument);
    if (handled) {
      setSlashOpen(false);
      onChange("");
    }
    return handled;
  }

  function insertSkill(skill: SkillInfo) {
    const prefix = activeTrigger?.prefix === "/" ? "/" : "@";
    insertCompletion(skillTagCompletion(prefix, skill.name));
  }

  function insertCompletion(completion: string) {
    const trigger = activeTrigger;
    const next = trigger
      ? replaceComposerAutocompleteTrigger(value, trigger, completion)
      : value.slice(0, cursor) + completion + value.slice(cursor);
    const nextCursor = (trigger?.start ?? cursor) + completion.length;
    setSlashDismissedValue(next);
    onChange(next);
    setSlashOpen(false);
    window.requestAnimationFrame(() => {
      ref.current?.focus();
      ref.current?.setSelectionRange(nextCursor, nextCursor);
      setCursor(nextCursor);
    });
  }

  function activateSuggestedCommand(commandId: string) {
    if (!onSlashCommand(commandId, "")) return;
    const trigger = activeTrigger;
    const next = trigger
      ? replaceComposerAutocompleteTrigger(value, trigger, "")
      : value;
    const nextCursor = trigger?.start ?? cursor;
    setSlashDismissedValue(next);
    onChange(next);
    setSlashOpen(false);
    window.requestAnimationFrame(() => {
      ref.current?.focus();
      ref.current?.setSelectionRange(nextCursor, nextCursor);
      setCursor(nextCursor);
    });
  }

  async function completeSuggestion(item: Suggestion) {
    if (item.kind === "action") {
      setSlashOpen(false);
      attachFromPicker();
      return;
    }
    if (item.kind === "file") {
      if (await onAttachWorkspaceFile(item.file)) {
        const path = item.file.path.includes(" ") ? `"${item.file.path}"` : item.file.path;
        insertCompletion(`@${path} `);
      }
      return;
    }
    if (item.kind === "skill") {
      recordSuggestionUse(`skill:${item.skill.id}`);
      insertSkill(item.skill);
      return;
    }
    if (item.kind === "mcp") {
      recordSuggestionUse(`mcp:${item.tool.name}`);
      insertCompletion(mcpToolTagCompletion(item.tool.name));
      return;
    }
    if (composerCommandRunsOnSelection(item.command.id)) {
      recordSuggestionUse(`command:${item.command.id}`);
      activateSuggestedCommand(item.command.id);
      return;
    }
    recordSuggestionUse(`command:${item.command.id}`);
    insertCompletion(`/${item.command.id} `);
  }

  function selectedSuggestion(): Suggestion | undefined {
    return orderedSuggestions[Math.min(slashFocusIndex, Math.max(0, orderedSuggestions.length - 1))];
  }

  function moveSlashFocus(step: number) {
    if (!orderedSuggestions.length) return;
    setSlashFocusIndex((index) => (index + step + orderedSuggestions.length) % orderedSuggestions.length);
  }

  function clearHistoryNoticeTimer() {
    if (historyNoticeTimerRef.current === null) return;
    window.clearTimeout(historyNoticeTimerRef.current);
    historyNoticeTimerRef.current = null;
  }

  function hideHistoryNotice() {
    clearHistoryNoticeTimer();
    setHistoryNotice(null);
  }

  function showHistoryNotice(index: number | null) {
    clearHistoryNoticeTimer();
    if (index === null) {
      setHistoryNotice(null);
      return;
    }
    setHistoryNotice(`History ${sentHistory.length - index} / ${sentHistory.length}`);
    historyNoticeTimerRef.current = window.setTimeout(() => {
      setHistoryNotice(null);
      historyNoticeTimerRef.current = null;
    }, COMPOSER_HISTORY_NOTICE_MS);
  }

  function recallHistory(direction: ComposerHistoryDirection, target: HTMLTextAreaElement): boolean {
    const currentIndex = historyIndex;
    if (!canNavigateComposerHistory(target.value, target.selectionStart, target.selectionEnd, direction, currentIndex)) return false;
    const draft = currentIndex === null ? target.value : historyDraftRef.current;
    const next = moveComposerHistory(sentHistory, draft, currentIndex, direction);
    if (!next) return false;
    if (currentIndex === null) historyDraftRef.current = target.value;
    applyingHistoryRef.current = next.value !== target.value;
    setHistoryIndex(next.index);
    showHistoryNotice(next.index);
    setSlashOpen(false);
    setSlashDismissedValue(next.value);
    onChange(next.value);
    window.requestAnimationFrame(() => {
      ref.current?.focus();
      ref.current?.setSelectionRange(next.value.length, next.value.length);
      setCursor(next.value.length);
    });
    return true;
  }

  function submitComposer() {
    completionControllerRef.current?.abort();
    setGhostCompletion("");
    if (!busy && slashInput && runSlash(slashInput.id, slashInput.argument)) return;
    onSend();
  }

  function applyAgent(agent: Agent | null) {
    onAgent(agent);
    setPersonaOpen(false);
  }

  function applyWorkspaceFolder(folder: string) {
    setProjectOpen(false);
    onWorkspaceFolder(folder);
  }

  function pickWorkspaceFolder() {
    setProjectOpen(false);
    onPickWorkspaceFolder();
  }

  function shouldSubmitFromEnter(event: KeyboardEvent<HTMLTextAreaElement>): boolean {
    if (event.key !== "Enter" || event.shiftKey) return false;
    if (composerSendShortcut === "enter") return true;
    return shortcutMatchesEvent("Mod+Enter", event);
  }

  return (
    <div
      className={`composer ${composerDensity === "compact" ? "compact" : "comfortable"}${dragOver ? " drag-over" : ""}`}
      data-testid="composer-drop-zone"
      onDragOver={(e) => {
        e.preventDefault();
        setDragOver(true);
      }}
      onDragLeave={() => setDragOver(false)}
      onDrop={(e) => {
        e.preventDefault();
        setDragOver(false);
        const files = Array.from(e.dataTransfer.files ?? []);
        if (files.length) onAttachFiles(files);
      }}
    >
      <input
        ref={fileRef}
        type="file"
        multiple
        hidden
        data-testid="composer-file-input"
        onChange={(e) => {
          const files = Array.from(e.currentTarget.files ?? []);
          e.currentTarget.value = "";
          if (files.length) onAttachFiles(files);
        }}
      />
      {attachments.length > 0 && (
        <div className="attachment-tray" data-testid="attachment-tray">
          {attachments.map((attachment) => (
            <span key={attachment.id} className="attachment-pill" data-testid={`attachment-pill-${attachment.id}`} title={attachment.name}>
              {attachment.dataUrl ? <img className="attachment-thumb" src={attachment.dataUrl} alt={`Attachment preview: ${attachment.name}`} /> : <Paperclip size={13} />}
              <span className="attachment-name">{attachment.name}</span>
              <span className="attachment-meta">
                {attachmentSizeLabel(attachment.size)}
                {attachment.truncated ? " clipped" : ""}
              </span>
              <button
                type="button"
                className="attachment-remove"
                title="Remove attachment"
                aria-label={`Remove attachment ${attachment.name}`}
                onClick={() => onRemoveAttachment(attachment.id)}
              >
                <X size={12} />
              </button>
            </span>
          ))}
        </div>
      )}
      <div className="composer-input-wrap">
        {showComposerOverlay && (
          <div ref={highlightRef} className="composer-input-highlight" aria-hidden="true" data-testid="composer-token-layer">
            {composerHighlightParts.map((part, index) =>
              part.kind === "token" ? (
                <span
                  key={`${part.token.kind}-${part.token.start}-${index}`}
                  className={`composer-token composer-token-${part.token.kind}`}
                  data-testid={`composer-token-${part.token.kind}`}
                >
                  {part.text}
                </span>
              ) : (
                <span key={`text-${index}`}>{part.text}</span>
              ),
            )}
            {ghostCompletion ? <span className="composer-ghost-text">{ghostCompletion}</span> : null}
            <span className="composer-highlight-sentinel">{"\u200b"}</span>
          </div>
        )}
        <textarea
          ref={ref}
          className={"composer-input" + (showComposerOverlay ? " has-token-layer" : "")}
          data-testid="composer-input"
          rows={1}
          value={value}
          dir="auto"
          placeholder={placeholder}
          onChange={(e) => {
            syncCursor(e.currentTarget);
            syncHighlightScroll(e.currentTarget);
            onChange(e.target.value);
          }}
          onCompositionStart={() => setImeComposing(true)}
          onCompositionEnd={() => setImeComposing(false)}
          onClick={(e) => syncCursor(e.currentTarget)}
          onKeyUp={(e) => syncCursor(e.currentTarget)}
          onSelect={(e) => syncCursor(e.currentTarget)}
          onScroll={(e) => syncHighlightScroll(e.currentTarget)}
          onKeyDown={(e) => {
            if (ghostCompletion && e.key === "Tab" && !e.shiftKey) {
              e.preventDefault();
              const next = value + ghostCompletion;
              setGhostCompletion("");
              onChange(next);
              window.requestAnimationFrame(() => ref.current?.setSelectionRange(next.length, next.length));
              return;
            }
            if (ghostCompletion && e.key === "Escape") {
              e.preventDefault();
              completionControllerRef.current?.abort();
              setGhostCompletion("");
              return;
            }
            if (e.key === "Escape" && showSlashMenu) {
              e.preventDefault();
              setSlashDismissedValue(value);
              setSlashOpen(false);
              return;
            }
            if (showSlashMenu && orderedSuggestions.length > 0) {
              if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                e.preventDefault();
                moveSlashFocus(e.key === "ArrowDown" ? 1 : -1);
                return;
              }
              if (e.key === "Enter" || e.key === "Tab") {
                e.preventDefault();
                const item = selectedSuggestion();
                if (item) void completeSuggestion(item);
                return;
              }
            }
            if ((e.key === "ArrowUp" || e.key === "ArrowDown") && !e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey) {
              if (recallHistory(e.key === "ArrowUp" ? "previous" : "next", e.currentTarget)) {
                e.preventDefault();
                return;
              }
            }
            if (shouldSubmitFromEnter(e)) {
              e.preventDefault();
              submitComposer();
            }
          }}
          onPaste={(e) => {
            const files = clipboardFiles(e.clipboardData);
            if (!files.length) return;
            e.preventDefault();
            onAttachFiles(files);
          }}
        />
      </div>
      {showSlashMenu && (
        <div className="slash-menu" data-testid="slash-menu">
          {(() => {
            let itemIndex = -1;
            return suggestionGroups.map((group) => (
            <div className="slash-group" key={group.label}>
              <div className="slash-group-label">{group.label}</div>
              {group.items.map((item) => {
                itemIndex += 1;
                const index = itemIndex;
                if (item.kind === "action") {
                  return (
                    <button
                      key={item.key}
                      type="button"
                      className={"slash-item" + (index === slashFocusIndex ? " active" : "")}
                      data-testid={`suggestion-${item.key}`}
                      onClick={() => {
                        setSlashOpen(false);
                        attachFromPicker();
                      }}
                    >
                      <span className="slash-name">{item.name}</span>
                      <span className="slash-label">{item.label}</span>
                      <span className="slash-hint">{item.hint}</span>
                    </button>
                  );
                }
                if (item.kind === "skill") {
                  return (
                    <button
                      key={item.skill.id}
                      type="button"
                      className={"slash-item" + (index === slashFocusIndex ? " active" : "")}
                      data-testid={`skill-suggestion-${item.skill.id}`}
                      onClick={() => void completeSuggestion(item)}
                    >
                      <span className="slash-name">{suggestionPrefix === "/" ? "/" : "@"}{item.skill.name}</span>
                      <span className="slash-label">{item.skill.name}</span>
                      <span className="slash-hint">{item.skill.description || "Use this skill"}</span>
                    </button>
                  );
                }
                if (item.kind === "mcp") {
                  return (
                    <button
                      key={item.tool.name}
                      type="button"
                      className={"slash-item" + (index === slashFocusIndex ? " active" : "")}
                      data-testid={`mcp-suggestion-${item.tool.name}`}
                      onClick={() => void completeSuggestion(item)}
                    >
                      <span className="slash-name">/{item.tool.name}</span>
                      <span className="slash-label">{item.tool.name}</span>
                      <span className="slash-hint">{item.tool.description || "Use this MCP tool"}</span>
                    </button>
                  );
                }
                if (item.kind === "file") {
                  return (
                    <button
                      key={item.file.full_path}
                      type="button"
                      className={"slash-item" + (index === slashFocusIndex ? " active" : "")}
                      data-testid={`file-suggestion-${item.file.path}`}
                      onClick={() => void completeSuggestion(item)}
                    >
                      <span className="slash-name">@{item.file.path}</span>
                      <span className="slash-label">{item.file.name}</span>
                      <span className="slash-hint">{attachmentSizeLabel(item.file.size)}</span>
                    </button>
                  );
                }
                return (
                  <button
                    key={item.command.id}
                    type="button"
                    className={"slash-item" + (index === slashFocusIndex ? " active" : "")}
                    data-testid={`slash-command-${item.command.id}`}
                    onClick={() =>
                      activeTrigger || composerCommandRunsOnSelection(item.command.id)
                        ? void completeSuggestion(item)
                        : runSlash(item.command.id, "")
                    }
                  >
                    <span className="slash-name">/{item.command.id}</span>
                    <span className="slash-label">{item.command.label}</span>
                    <span className="slash-hint">{item.command.placeholder ?? item.command.hint}</span>
                  </button>
                );
              })}
            </div>
            ));
          })()}
        </div>
      )}
      <div className="composer-bar">
        <div className="composer-tools">
          {showWorkspaceSelector && (
            <div className="project-chip-wrap" ref={projectRef}>
              <button
                type="button"
                className={"project-chip" + (activeWorkspaceFolder ? " active" : "")}
                data-testid="composer-project-selector"
                title={activeWorkspaceFolder || "Don't work in a project"}
                aria-label={`Project, current ${activeWorkspaceLabel}`}
                aria-expanded={projectOpen}
                aria-haspopup="menu"
                onClick={() => {
                  setProjectOpen((open) => !open);
                  setSlashOpen(false);
                  setPersonaOpen(false);
                }}
              >
                <Folder size={14} />
                <span>{activeWorkspaceLabel}</span>
                <ChevronDown size={12} />
              </button>
              {projectOpen && (
                <div className="menu project-chip-menu" role="menu" aria-label="Project">
                  <button
                    type="button"
                    role="menuitem"
                    className={"menu-item" + (!activeWorkspaceFolder ? " active" : "")}
                    data-testid="composer-project-none"
                    onClick={() => applyWorkspaceFolder("")}
                  >
                    <Folder size={14} />
                    <span>Don't work in a project</span>
                  </button>
                  {workspaceProjects.length > 0 && <div className="menu-sep" />}
                  {workspaceProjects.map((project) => (
                    <button
                      type="button"
                      role="menuitem"
                      key={project.folder}
                      className={"menu-item" + (project.folder === activeWorkspaceFolder ? " active" : "")}
                      title={project.folder}
                      onClick={() => applyWorkspaceFolder(project.folder)}
                    >
                      <Folder size={14} />
                      <span className="project-chip-menu-name">{project.name}</span>
                    </button>
                  ))}
                  <div className="menu-sep" />
                  <button type="button" role="menuitem" className="menu-item" onClick={pickWorkspaceFolder}>
                    <FolderOpen size={14} />
                    <span>Choose folder...</span>
                  </button>
                </div>
              )}
            </div>
          )}
          <button className="tool-btn" data-testid="attach-files" title="Attach files" aria-label="Attach files" onClick={attachFromPicker}>
            <Paperclip size={16} />
          </button>
          <button
            className="tool-btn"
            data-testid="slash-command-trigger"
            title="Commands and skills"
            aria-label="Commands and skills"
            aria-expanded={showSlashMenu}
            aria-haspopup="menu"
            onClick={() => {
              if (autocompleteMode !== "off") setSlashOpen((v) => !v);
              setPersonaOpen(false);
            }}
          >
            <Slash size={16} />
          </button>
          <div className="persona-wrap" ref={personaRef}>
            <button
              className={"tool-btn persona-btn" + (activeAgent || hasInstr ? " active" : "")}
              data-testid="agent-switcher"
              title="Persona"
              aria-label={`Persona, current ${activeAgent?.name ?? "Default chat"}`}
              aria-expanded={personaOpen}
              aria-haspopup="dialog"
              onClick={() => {
                setPersonaOpen((v) => !v);
                setSlashOpen(false);
              }}
            >
              {activeAgent ? (
                <AgentAvatar id={activeAgent.id} name={activeAgent.name} avatar={activeAgent.avatar} />
              ) : (
                <span className="agent-badge" aria-hidden="true"><UserRound size={13} /></span>
              )}
            </button>
            {personaOpen && (
              <div className="menu menu-wide chat-menu persona-menu">
                <div className="menu-title">Persona</div>
                <button type="button" className={"menu-item" + (!activeAgentId ? " active" : "")} onClick={() => applyAgent(null)}>
                  <span className="agent-badge" aria-hidden="true"><UserRound size={13} /></span>
                  <span>Default chat</span>
                </button>
                {agents.map((agent) => (
                  <button
                    type="button"
                    key={agent.id}
                    className={"menu-item" + (agent.id === activeAgentId ? " active" : "")}
                    data-testid={`agent-option-${agent.name}`}
                    onClick={() => applyAgent(agent)}
                  >
                    <AgentAvatar id={agent.id} name={agent.name} avatar={agent.avatar} />
                    <span className="agent-mono">{agent.name}</span>
                    <span className="agent-model">{agentMenuDetail(agent)}</span>
                  </button>
                ))}
                <button
                  type="button"
                  className="menu-item"
                  data-testid="manage-agents"
                  onClick={() => {
                    setPersonaOpen(false);
                    onManageAgents();
                  }}
                >
                  <PlusSquare size={15} />
                  <span>Manage agents...</span>
                </button>
                <div className="menu-sep" />
                <div className="menu-title">Instructions</div>
                <textarea
                  className="instr-input"
                  value={instructions}
                  placeholder="System prompt the assistant always follows..."
                  onChange={(e) => onInstructions(e.target.value)}
                />
                {hasInstr && (
                  <button type="button" className="menu-clear" onClick={() => onInstructions("")}>
                    Clear
                  </button>
                )}
              </div>
            )}
          </div>
        </div>
        <div className="composer-send">
          <span
            className={"composer-status" + (historyNotice ? " history" : "") + (busy ? " shiny-text" : "")}
            data-testid={historyNotice ? "composer-history-status" : undefined}
            role="status"
            aria-live="polite"
            title={historyNotice ?? undefined}
          >
            {busy
              ? "generating..."
              : historyNotice
                ? historyNotice
                : mediaActive
                  ? mediaTargetLabel ?? `${mediaKind} mode`
                  : contextTokenLabel}
          </span>
          {busy ? (
            <>
              <button className="send-btn" data-testid="composer-send" onClick={submitComposer} disabled={!canSend} title={`Queue (${sendShortcutLabel})`} aria-label="Queue message">
                <ArrowUp size={18} />
              </button>
              <button className="send-btn stop" onClick={onStop} title="Stop generating" aria-label="Stop generating">
                <Square size={13} />
              </button>
            </>
          ) : (
            <button className="send-btn" data-testid="composer-send" onClick={submitComposer} disabled={!canSend} title={`Send (${sendShortcutLabel})`} aria-label="Send message">
              <ArrowUp size={18} />
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
