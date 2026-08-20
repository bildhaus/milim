import { useEffect, useMemo, useState } from "react";
import "../settings.css";
import { useAgents } from "../agents/store";
import {
  deleteAgent,
  generateAgentDraft,
  isAgentDraftModel,
  isLegacyAgentAvatar,
  listSchedules,
  listSkills,
  listTools,
  saveAgent,
  streamAgentRun,
  type Agent,
  type AgentEvent,
  type AgentSkillMode,
  type AgentToolMode,
  type ScheduleInfo,
  type SkillInfo,
  type ToolInfo,
} from "../api";
import { useSessions } from "../sessions/store";
import { createCanonicalChat } from "../lib/newChatCoordinator";
import { AgentAvatar } from "./AgentAvatar";
import { Calendar, Copy, Plus, Sparkles, Trash, X } from "./icons";
import { SheetDialog } from "./SheetDialog";
import { Checkbox } from "./ui";
import "./AgentsManager.css";

type Selection = Agent | "new" | null;
type ToolGroupName = "Files" | "Shell" | "Web" | "Computer" | "MCP" | "Other";

interface ToolGroupMeta {
  group: ToolGroupName;
  detail: string;
}

interface AgentStarter {
  key: string;
  name: string;
  avatar: string;
  description: string;
  systemPrompt: string;
  toolMode: AgentToolMode;
  toolGroups?: ToolGroupName[];
}

const TOOL_GROUPS: ToolGroupMeta[] = [
  { group: "Files", detail: "Read and write scoped project files." },
  { group: "Shell", detail: "Run command and sandbox workflows." },
  { group: "Web", detail: "Fetch pages and remote context." },
  { group: "Computer", detail: "Use screen, mouse, and keyboard tools." },
  { group: "MCP", detail: "Call connected MCP server tools." },
  { group: "Other", detail: "Utility and app-specific tools." },
];

const AGENT_STARTERS: AgentStarter[] = [
  {
    key: "code-reviewer",
    name: "Code review lead",
    avatar: "CR",
    description: "Reviews local changes, risky diffs, tests, regressions, and security issues before merge.",
    toolMode: "custom",
    toolGroups: ["Files", "Shell"],
    systemPrompt: [
      "You are a Milim code review lead working inside a local-first developer harness.",
      "Use the selected workspace folder, file tools, search tools, git context, and guarded shell tools when they are available. Inspect before judging; do not invent files, diffs, test output, or line numbers.",
      "Your job is to find correctness bugs, regressions, data-loss risks, security issues, broken edge cases, and missing verification. Style opinions are secondary unless they hide a real maintainability risk.",
      "When reviewing code, trace the actual flow end to end before proposing changes. Prefer one root-cause fix in the shared path over caller-by-caller patches.",
      "Respect Milim's tool approval, sandbox, privacy, and workspace limits. If a tool is unavailable, say what evidence is missing instead of pretending you checked it.",
      "Use the run timeline as the audit trail: keep tool calls purposeful, avoid noisy commands, and summarize the result of every meaningful check.",
      "Output format: findings first, ordered by severity, with file references when available. Then list test gaps or verification run. Keep the final summary short.",
      "If there are no findings, say that clearly and mention the strongest evidence used to reach that conclusion.",
    ].join("\n"),
  },
  {
    key: "web-researcher",
    name: "Web research scout",
    avatar: "WR",
    description: "Uses Milim web tools to gather current sources, dates, links, and evidence.",
    toolMode: "custom",
    toolGroups: ["Web"],
    systemPrompt: [
      "You are a Milim web research scout for current, source-backed answers.",
      "Use web/fetch tools when facts may have changed, when the user asks for latest information, or when direct attribution matters. Prefer primary sources, official docs, original announcements, standards, papers, or vendor pages.",
      "Track dates carefully. Distinguish publication date, event date, and the current date when that affects the answer.",
      "Separate facts from inference. If a conclusion is synthesized from multiple sources, label it as an inference and explain the evidence in one sentence.",
      "Keep Milim privacy boundaries: do not send private local content to remote sources unless the user explicitly asked for that workflow.",
      "When sources disagree, report the conflict instead of smoothing it over.",
      "Output format: short answer first, then evidence bullets with source names and links, then open uncertainties if any.",
      "Avoid long quotes. Paraphrase aggressively and quote only short phrases when exact wording matters.",
    ].join("\n"),
  },
  {
    key: "local-operator",
    name: "Local project operator",
    avatar: "LO",
    description: "Uses workspace, file, git, shell, sandbox, and MCP tools to make scoped local progress.",
    toolMode: "custom",
    toolGroups: ["Files", "Shell"],
    systemPrompt: [
      "You are a Milim local project operator.",
      "Treat the selected folder as the source of truth. Read the relevant files, configs, tests, and docs before acting. Reuse existing project patterns and helpers.",
      "Prefer the smallest working change that moves the task forward. Do not create abstractions, dependencies, config systems, or scaffolding unless the repo already points that way.",
      "Use file tools for inspection, git-aware commands for current state, and shell/sandbox tools for verification. Respect tool approval and never run destructive commands unless explicitly requested.",
      "If MCP tools are available, use them only when they are the direct fit for the task. Do not call tools just to look busy.",
      "For non-trivial edits, leave one runnable verification: a focused test, build, lint, or self-check. If verification cannot run, explain why.",
      "Output format: what changed, verification, and any residual risk. Keep it terse.",
      "If the task is ambiguous but a safe default exists, choose the default and note the assumption.",
    ].join("\n"),
  },
  {
    key: "prompt-editor",
    name: "Prompt systems editor",
    avatar: "PE",
    description: "Turns rough instructions into durable prompts, agent profiles, and skill text.",
    toolMode: "none",
    systemPrompt: [
      "You are a Milim prompt systems editor.",
      "Rewrite rough instructions into durable system prompts, reusable agent profiles, skill instructions, or task prompts that can run inside Milim.",
      "Preserve the user's real intent, voice, constraints, and edge cases. Remove ambiguity, contradictions, vague success criteria, hidden assumptions, and untestable wording.",
      "When writing an agent profile, include role, operating context, tool behavior, boundaries, output format, and verification expectations.",
      "When writing a skill, include when to use it, when not to use it, and the exact behavior the assistant should follow.",
      "Do not add tool access, provider assumptions, or product claims unless the user asked for them. Prefer clear defaults over long option lists.",
      "Output directly usable text. Avoid meta commentary unless the user asked for analysis.",
      "Ask only when a missing choice would materially change the final prompt.",
    ].join("\n"),
  },
  {
    key: "research-analyst",
    name: "Research analyst",
    avatar: "RA",
    description: "Combines Milim memory, local files, web evidence, and tradeoffs into decisions.",
    toolMode: "custom",
    toolGroups: ["Files", "Web"],
    systemPrompt: [
      "You are a Milim research analyst for technical and product decisions.",
      "Use local project files, Milim memory, workspace context, and web sources when each is relevant. Do not treat memory as proof; use it as context to verify against current files or sources.",
      "Frame the decision before researching: objective, constraints, options, decision owner, and what evidence would change the recommendation.",
      "Compare options against the user's actual stack, privacy/local-first preference, multi-provider AI setup, and cross-platform targets when relevant.",
      "Keep the analysis decision-oriented. Avoid giant literature reviews unless the user asks for one.",
      "When evidence is weak, say so plainly and recommend the cheapest next check.",
      "Output format: recommendation, why, tradeoffs, evidence, and next action.",
      "If the right answer is to not build something, say that and explain the simpler alternative.",
    ].join("\n"),
  },
  {
    key: "documentation-maintainer",
    name: "Documentation maintainer",
    avatar: "DM",
    description: "Keeps READMEs, guides, examples, and behavior docs accurate as the product changes.",
    toolMode: "custom",
    toolGroups: ["Files", "Shell"],
    systemPrompt: [
      "You are a Milim documentation maintainer for software projects.",
      "Treat the current code, tests, configuration, and shipped behavior as the source of truth. Read the repository's documentation conventions before editing.",
      "Keep top-level overviews concise and put detailed behavior in the repository's existing documentation structure. Reuse its terminology, voice, formatting, and link style.",
      "Update only documentation affected by the change. Do not rewrite unrelated sections or duplicate content across files.",
      "Verify commands, paths, examples, links, and product claims against the project. Never document speculative behavior as shipped.",
      "Use file and shell tools when available to find stale references and run the smallest relevant documentation check.",
      "Output format: changed documentation, verification, and any behavior that remains undocumented because evidence was missing.",
    ].join("\n"),
  },
];

function toolGroup(tool: ToolInfo): ToolGroupName {
  const name = tool.name.toLowerCase();
  if (name.includes("__")) return "MCP";
  if (name.includes("file") || name.includes("dir") || name.includes("workspace")) return "Files";
  if (name.includes("shell") || name.includes("command") || name.includes("sandbox")) return "Shell";
  if (name.includes("http") || name.includes("fetch") || name.includes("web")) return "Web";
  if (name.includes("screen") || name.includes("mouse") || name.includes("key") || name.includes("scroll") || name.includes("type_text")) {
    return "Computer";
  }
  return "Other";
}

function plural(value: number, unit: string): string {
  return `${value} ${unit}${value === 1 ? "" : "s"}`;
}

function agentToolSummary(agent: Agent): string {
  const mode = agent.tool_mode ?? ((agent.enabled_tools ?? []).length === 0 ? "all" : "custom");
  if (mode === "all") return "All tools";
  if (mode === "none") return "No tools";
  return `${agent.enabled_tools?.length ?? 0} tools`;
}

function agentSkillSummary(agent: Agent): string {
  const mode = agent.skill_mode ?? ((agent.enabled_skills ?? []).length === 0 ? "auto" : "custom");
  if (mode === "auto") return "Auto skills";
  if (mode === "none") return "No skills";
  return `${agent.enabled_skills?.length ?? 0} skills`;
}

function draftToolSummary(mode: AgentToolMode, count: number): string {
  if (mode === "all") return "All tools";
  if (mode === "none") return "No tools";
  return `${count} selected`;
}

function draftSkillSummary(mode: AgentSkillMode, count: number): string {
  if (mode === "auto") return "Auto skills";
  if (mode === "none") return "No skills";
  return `${count} selected`;
}

function agentUsageSummary(isActive: boolean, scheduleCount: number): string {
  const parts = [];
  if (isActive) parts.push("Active chat");
  if (scheduleCount > 0) parts.push(plural(scheduleCount, "schedule"));
  return parts.join(" + ");
}

function runtimeToolDetail(mode: AgentToolMode, selectedCount: number, availableCount: number): string {
  if (mode === "all") return availableCount ? `${availableCount} available at run time` : "All registered tools when available";
  if (mode === "none") return "No tool schemas sent to the model";
  return selectedCount ? `${selectedCount} selected tools` : "Select at least one tool";
}

function runtimeSkillDetail(mode: AgentSkillMode, selectedCount: number, availableCount: number): string {
  if (mode === "auto") return availableCount ? "Milim picks matching enabled skills" : "No installed skills yet";
  if (mode === "none") return "No skill instructions are injected";
  return selectedCount ? `${selectedCount} pinned skills` : "Select at least one skill";
}

function AgentListPlaceholder() {
  return (
    <div className="agent-list-placeholder">
      <span>No agents</span>
    </div>
  );
}

function AgentStarterGrid({
  onStarter,
  collapsed = false,
}: {
  onStarter: (starter: AgentStarter) => void;
  collapsed?: boolean;
}) {
  return (
    <div className="agent-starter-grid" aria-label="Agent starters">
      {AGENT_STARTERS.map((starter) => (
        <button
          className="agent-starter-card"
          key={starter.key}
          type="button"
          title={starter.description}
          aria-label={`${starter.name}: ${starter.description}`}
          tabIndex={collapsed ? -1 : 0}
          onClick={() => onStarter(starter)}
        >
          <AgentAvatar name={starter.name} avatar={starter.avatar} className="agent-starter-badge" />
          <span className="agent-starter-name">{starter.name}</span>
        </button>
      ))}
    </div>
  );
}

export function AgentsManager({ onClose }: { onClose: () => void }) {
  const agents = useAgents((s) => s.agents);
  const refresh = useAgents((s) => s.refresh);
  const activeSession = useSessions((s) => s.sessions.find((x) => x.id === s.activeId));
  const currentThreadModel = activeSession?.settings?.model ?? "";
  const activeAgentId = activeSession?.settings?.activeAgentId ?? null;

  const [tools, setTools] = useState<ToolInfo[]>([]);
  const [schedules, setSchedules] = useState<ScheduleInfo[]>([]);
  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [bootstrapped, setBootstrapped] = useState(false);
  const [sel, setSel] = useState<Selection>(null);

  const [draftPrompt, setDraftPrompt] = useState("");
  const [draftGenerating, setDraftGenerating] = useState(false);
  const [draftError, setDraftError] = useState("");
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [instructions, setInstructions] = useState("");
  const [enabled, setEnabled] = useState<string[]>([]);
  const [avatar, setAvatar] = useState("");
  const [toolMode, setToolMode] = useState<AgentToolMode>("all");
  const [skillMode, setSkillMode] = useState<AgentSkillMode>("auto");
  const [enabledSkills, setEnabledSkills] = useState<string[]>([]);
  const [toolSearch, setToolSearch] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [testPrompt, setTestPrompt] = useState("");
  const [testResult, setTestResult] = useState("");
  const [testError, setTestError] = useState("");
  const [testRunning, setTestRunning] = useState(false);

  useEffect(() => {
    let alive = true;
    listTools().then((next) => alive && setTools(next));
    listSchedules().then((next) => alive && setSchedules(next));
    listSkills().then((next) => alive && setSkills(next));
    refresh().finally(() => {
      if (alive) setBootstrapped(true);
    });
    return () => {
      alive = false;
    };
  }, [refresh]);

  useEffect(() => {
    if (bootstrapped && agents.length === 0 && sel === null) edit("new");
  }, [agents.length, bootstrapped, sel]);

  const groupedTools = useMemo(() => {
    const q = toolSearch.trim().toLowerCase();
    const visible = tools.filter((t) => {
      if (!q) return true;
      return t.name.toLowerCase().includes(q) || t.description.toLowerCase().includes(q);
    });
    return TOOL_GROUPS.map(({ group }) => ({
      group,
      tools: visible.filter((t) => toolGroup(t) === group),
    })).filter((g) => g.tools.length > 0);
  }, [toolSearch, tools]);

  const toolCapabilities = useMemo(
    () =>
      TOOL_GROUPS.map((meta) => {
        const groupTools = tools.filter((t) => toolGroup(t) === meta.group);
        return {
          ...meta,
          total: groupTools.length,
          selected: groupTools.filter((t) => enabled.includes(t.name)).length,
        };
      }).filter((item) => item.total > 0),
    [enabled, tools],
  );

  const scheduleCountsByAgent = useMemo(() => {
    const counts = new Map<string, number>();
    for (const schedule of schedules) {
      if (schedule.agent_id) counts.set(schedule.agent_id, (counts.get(schedule.agent_id) ?? 0) + 1);
    }
    return counts;
  }, [schedules]);

  function resetTest() {
    setTestPrompt("");
    setTestResult("");
    setTestError("");
    setTestRunning(false);
  }

  function edit(a: Agent | "new") {
    setSel(a);
    setToolSearch("");
    setDraftPrompt("");
    setDraftError("");
    setConfirmDeleteId(null);
    resetTest();
    if (a === "new") {
      setName("");
      setDescription("");
      setInstructions("");
      setEnabled([]);
      setAvatar("");
      setToolMode("all");
      setSkillMode("auto");
      setEnabledSkills([]);
    } else {
      setName(a.name);
      setDescription(a.description ?? "");
      setInstructions(a.system_prompt);
      setEnabled(a.enabled_tools ?? []);
      const storedAvatar = a.avatar ?? "";
      setAvatar(isLegacyAgentAvatar(storedAvatar) ? "" : storedAvatar);
      setToolMode(a.tool_mode ?? ((a.enabled_tools ?? []).length === 0 ? "all" : "custom"));
      setSkillMode(a.skill_mode ?? ((a.enabled_skills ?? []).length === 0 ? "auto" : "custom"));
      setEnabledSkills(a.enabled_skills ?? []);
    }
  }

  function toolsForGroups(groups: ToolGroupName[] | undefined): string[] {
    if (!groups?.length) return [];
    return tools.filter((tool) => groups.includes(toolGroup(tool))).map((tool) => tool.name);
  }

  function applyStarter(starter: AgentStarter) {
    const starterTools = toolsForGroups(starter.toolGroups);
    setSel("new");
    setToolSearch("");
    setDraftPrompt("");
    setDraftError("");
    setConfirmDeleteId(null);
    resetTest();
    setName(starter.name);
    setDescription(starter.description);
    setInstructions(starter.systemPrompt);
    setAvatar(starter.avatar);
    setToolMode(starter.toolMode === "custom" && starterTools.length === 0 ? "none" : starter.toolMode);
    setEnabled(starter.toolMode === "custom" ? starterTools : []);
    setSkillMode("auto");
    setEnabledSkills([]);
  }

  function duplicateAgent(agent: Agent) {
    setSel("new");
    setToolSearch("");
    setDraftPrompt("");
    setDraftError("");
    setConfirmDeleteId(null);
    resetTest();
    setName(`${agent.name} copy`);
    setDescription(agent.description ?? "");
    setInstructions(agent.system_prompt);
    setEnabled(agent.enabled_tools ?? []);
    const storedAvatar = agent.avatar ?? "";
    setAvatar(isLegacyAgentAvatar(storedAvatar) ? "" : storedAvatar);
    setToolMode(agent.tool_mode ?? ((agent.enabled_tools ?? []).length === 0 ? "all" : "custom"));
    setSkillMode(agent.skill_mode ?? ((agent.enabled_skills ?? []).length === 0 ? "auto" : "custom"));
    setEnabledSkills(agent.enabled_skills ?? []);
  }

  async function draftAgent() {
    const prompt = draftPrompt.trim();
    if (!prompt || draftGenerating || !generationModel) return;
    setDraftError("");
    setDraftGenerating(true);
    try {
      const draft = await generateAgentDraft(prompt, generationModel);
      setName(draft.name);
      setDescription(draft.description);
      setAvatar(draft.avatar);
      setInstructions(draft.system_prompt);
    } catch (error) {
      setDraftError(error instanceof Error ? error.message : "Could not draft an agent.");
    } finally {
      setDraftGenerating(false);
    }
  }

  async function testAgent() {
    if (!selectedAgent) return;
    const prompt = testPrompt.trim();
    if (!prompt || testRunning) return;
    const requestModel = currentThreadModel.trim();
    if (!requestModel) {
      setTestError("Choose a model in the current chat before testing.");
      return;
    }
    let output = "";
    let streamError = "";
    setTestRunning(true);
    setTestError("");
    setTestResult("");
    try {
      await streamAgentRun(
        selectedAgent.id,
        requestModel,
        [{ role: "user", content: prompt }],
        (ev: AgentEvent) => {
          if (ev.type === "token" && ev.text) {
            output += ev.text;
            setTestResult(output);
          } else if (ev.type === "final" && (ev.content || ev.text) && !output) {
            output = ev.content || ev.text || "";
            setTestResult(output);
          } else if (ev.type === "error") {
            streamError = ev.message || "Agent test failed.";
          }
        },
        undefined,
        { memory_enabled: false },
        {
          tool_approval_policy: "review",
          tool_approval_grant: false,
          sandbox_enabled: false,
          computer_use_enabled: false,
          experimental_hashline_patch: false,
        },
      );
      if (streamError) throw new Error(streamError);
      if (!output.trim()) setTestResult("No text returned.");
    } catch (error) {
      setTestError(error instanceof Error ? error.message : String(error));
    } finally {
      setTestRunning(false);
    }
  }

  async function save() {
    if (!name.trim()) return;
    const id = sel && sel !== "new" ? sel.id : undefined;
    setConfirmDeleteId(null);
    const saved = await saveAgent({
      id,
      name: name.trim(),
      description: description.trim(),
      model: "",
      system_prompt: instructions,
      tool_mode: toolMode,
      enabled_tools: toolMode === "custom" ? enabled : [],
      skill_mode: skillMode,
      enabled_skills: skillMode === "custom" ? enabledSkills : [],
      avatar: avatar.trim(),
    });
    await refresh();
    if (saved) setSel(saved);
  }

  async function remove() {
    if (!sel || sel === "new") return;
    if (confirmDeleteId !== sel.id) {
      setConfirmDeleteId(sel.id);
      return;
    }
    await deleteAgent(sel.id);
    await refresh();
    setConfirmDeleteId(null);
    setSel(null);
  }

  function startChat(agent: Agent) {
    void createCanonicalChat({ activeAgentId: agent.id }).then(onClose);
  }

  const toggleTool = (t: string) =>
    setEnabled((e) => (e.includes(t) ? e.filter((x) => x !== t) : [...e, t]));

  const setToolModeValue = (value: AgentToolMode) => {
    setToolMode(value);
    if (value === "custom" && enabled.length === 0) setEnabled(tools.map((t) => t.name));
  };

  const setToolGroupEnabled = (group: ToolGroupName, shouldEnable: boolean) => {
    const names = tools.filter((tool) => toolGroup(tool) === group).map((tool) => tool.name);
    setEnabled((current) => {
      if (shouldEnable) return Array.from(new Set([...current, ...names]));
      const blocked = new Set(names);
      return current.filter((item) => !blocked.has(item));
    });
  };

  const toggleSkill = (id: string) =>
    setEnabledSkills((current) => (current.includes(id) ? current.filter((item) => item !== id) : [...current, id]));

  const canSave =
    name.trim().length > 0 &&
    (toolMode !== "custom" || enabled.length > 0) &&
    (skillMode !== "custom" || enabledSkills.length > 0);
  const selectedAgent = sel && sel !== "new" ? sel : null;
  const selectedScheduleCount = selectedAgent ? scheduleCountsByAgent.get(selectedAgent.id) ?? 0 : 0;
  const selectedUsage = selectedAgent ? agentUsageSummary(selectedAgent.id === activeAgentId, selectedScheduleCount) : "";
  const enabledToolCount = toolMode === "custom" ? enabled.length : toolMode === "all" ? tools.length : 0;
  const editorTitle = sel === "new" ? "New agent" : name.trim() || selectedAgent?.name || "Select an agent";
  const generationModel = isAgentDraftModel(currentThreadModel) ? currentThreadModel : "";
  const canDraftAgent = draftPrompt.trim().length > 0 && Boolean(generationModel) && !draftGenerating;
  const hasDraftContent = Boolean(
    name.trim() ||
      description.trim() ||
      instructions.trim() ||
      avatar.trim() ||
      enabled.length ||
      toolMode !== "all" ||
      enabledSkills.length ||
      skillMode !== "auto",
  );
  const canTestAgent = Boolean(selectedAgent && testPrompt.trim() && !testRunning);
  const enabledSkillList = skills.filter((skill) => skill.enabled);
  const deleteNote = selectedScheduleCount
    ? `Click again to delete. ${plural(selectedScheduleCount, "schedule")} will keep a missing agent reference.`
    : "Click again to delete this agent.";

  return (
    <SheetDialog title="Agents" className="sheet agents-sheet agent-manager-sheet" onClose={onClose}>
        <div className="agent-manager-header">
          <div className="agent-manager-title">
            <h2>Agents</h2>
            <p>Save reusable system prompts, avatars, skills, and tool access profiles.</p>
          </div>
          <div className="agent-manager-header-actions">
            <button className="btn-accent agent-header-action" data-testid="new-agent" type="button" onClick={() => edit("new")}>
              <Plus size={14} />
              <span>New agent</span>
            </button>
            <button className="icon-btn sheet-close agent-close" data-testid="close-agents" type="button" onClick={onClose} title="Close" aria-label="Close agents">
              <X size={15} />
            </button>
          </div>
        </div>

        <div className="agent-manager-body">
          <aside className="agent-rail" aria-label="Saved agents">
            <div className="agent-rail-summary">
              <span>{agents.length} saved</span>
              <span>{schedules.length} schedules</span>
            </div>
            <button className="agent-rail-action" type="button" onClick={() => edit("new")}>
              <Plus size={14} />
              <span>New</span>
            </button>
            {agents.length > 0 ? (
              <div className="agent-list" role="list">
                {agents.map((a) => {
                  const usage = agentUsageSummary(a.id === activeAgentId, scheduleCountsByAgent.get(a.id) ?? 0);
                  return (
                    <button
                      key={a.id}
                      className={"agent-list-card" + (selectedAgent?.id === a.id ? " active" : "")}
                      data-testid={`agent-editor-${a.name}`}
                      type="button"
                      onClick={() => edit(a)}
                    >
                      <AgentAvatar id={a.id} name={a.name} avatar={a.avatar} className="agent-card-badge" />
                      <span className="agent-card-copy">
                        <span className="agent-card-name">{a.name}</span>
                        {a.description && <span className="agent-card-description">{a.description}</span>}
                        <span className="agent-card-meta">
                          <span>{agentToolSummary(a)}</span>
                          <span>{agentSkillSummary(a)}</span>
                          {usage && <span>{usage}</span>}
                        </span>
                      </span>
                    </button>
                  );
                })}
              </div>
            ) : (
              <AgentListPlaceholder />
            )}
          </aside>

          <section className="agent-editor-panel" aria-label="Agent editor">
            {sel ? (
              <div className="agent-editor">
                <div className="agent-editor-head">
                  <div>
                    <span className="agent-editor-kicker">{sel === "new" ? "New profile" : "Saved profile"}</span>
                    <h3>{editorTitle}</h3>
                  </div>
                  {(sel !== "new" || hasDraftContent) && (
                    <span className="agent-editor-state">{draftToolSummary(toolMode, enabled.length)}</span>
                  )}
                </div>

                {(sel !== "new" || hasDraftContent) && (
                  <div className="agent-impact-panel" aria-label="Run impact">
                    <div className="agent-impact-item">
                      <span>Prompt</span>
                      <strong>{instructions.trim() ? `${instructions.trim().length} chars` : "No system prompt"}</strong>
                      <em>{instructions.trim() ? "Prepended server-side" : "Default chat behavior"}</em>
                    </div>
                    <div className="agent-impact-item">
                      <span>Skills</span>
                      <strong>{draftSkillSummary(skillMode, enabledSkills.length)}</strong>
                      <em>{runtimeSkillDetail(skillMode, enabledSkills.length, enabledSkillList.length)}</em>
                    </div>
                    <div className="agent-impact-item">
                      <span>Tools</span>
                      <strong>{draftToolSummary(toolMode, enabled.length)}</strong>
                      <em>{runtimeToolDetail(toolMode, enabled.length, tools.length)}</em>
                    </div>
                  </div>
                )}

                <section className="agent-editor-section agent-draft-section">
                  <div className="agent-section-head">
                    <h4>Draft with AI</h4>
                    <span>{draftGenerating ? "Drafting" : generationModel || "Choose a chat model first"}</span>
                  </div>
                  <div className="agent-draft-panel">
                    <div className="agent-draft-input-row">
                      <textarea
                        className="instr-input agent-draft-prompt"
                        data-testid="agent-draft-prompt"
                        rows={1}
                        value={draftPrompt}
                        onChange={(e) => {
                          setDraftPrompt(e.target.value);
                          if (draftError) setDraftError("");
                        }}
                        placeholder="Describe the agent to draft"
                      />
                      <button
                        className="btn-accent agent-draft-button"
                        data-testid="generate-agent-draft"
                        type="button"
                        disabled={!canDraftAgent}
                        onClick={draftAgent}
                        title={generationModel ? "Draft agent" : "Choose a model or provider first"}
                      >
                        <Sparkles size={14} />
                        <span>{draftGenerating ? "Drafting..." : "Draft"}</span>
                      </button>
                    </div>
                    {draftError && (
                      <span className="agent-draft-error" data-testid="agent-draft-error">
                        {draftError}
                      </span>
                    )}
                  </div>
                </section>

                {sel === "new" && (
                  <section
                    className={`agent-editor-section agent-starters-section${hasDraftContent ? " collapsed" : ""}`}
                    aria-hidden={hasDraftContent}
                  >
                    <div className="agent-starters-reveal">
                      <div className="agent-section-head">
                        <h4>Built-in profiles</h4>
                      </div>
                      <AgentStarterGrid onStarter={applyStarter} collapsed={hasDraftContent} />
                    </div>
                  </section>
                )}

                <section className="agent-editor-section">
                  <div className="agent-section-head">
                    <h4>Identity</h4>
                  </div>
                  <div className="agent-identity-row">
                    <AgentAvatar id={selectedAgent?.id} name={name} avatar={avatar} className="agent-identity-avatar" />
                    <label className="field agent-field">
                      <span>Name</span>
                      <input
                        className="css-input"
                        data-testid="agent-name-input"
                        value={name}
                        onChange={(e) => setName(e.target.value)}
                        placeholder="Researcher"
                      />
                    </label>
                    <label className="field agent-field agent-seed-field">
                      <span>Seed · optional</span>
                      <input
                        className="css-input"
                        data-testid="agent-avatar-input"
                        value={avatar}
                        onChange={(e) => setAvatar(e.target.value)}
                        placeholder="researcher"
                        title="The same seed keeps the same avatar. Leave blank to follow the name."
                      />
                    </label>
                  </div>
                  <label className="field agent-field agent-description-field">
                    <span>Description</span>
                    <input
                      className="css-input"
                      data-testid="agent-description-input"
                      value={description}
                      onChange={(e) => setDescription(e.target.value)}
                      placeholder="When should someone choose this agent?"
                      maxLength={240}
                    />
                  </label>
                </section>

                <section className="agent-editor-section">
                  <div className="agent-section-head">
                    <h4>System prompt</h4>
                    <span>{instructions.trim().length} chars</span>
                  </div>
                  <textarea
                    className="instr-input agent-instructions"
                    data-testid="agent-system-prompt"
                    value={instructions}
                    onChange={(e) => setInstructions(e.target.value)}
                    placeholder="You are a meticulous research assistant..."
                  />
                </section>

                <section className="agent-editor-section">
                  <div className="agent-section-head">
                    <h4>Skills</h4>
                    <span>{draftSkillSummary(skillMode, enabledSkills.length)}</span>
                  </div>
                  <div className="tool-panel agent-skill-panel">
                    <div className="tool-panel-head">
                      <div>
                        <strong>Skill instructions</strong>
                        <span>
                          {skillMode === "auto" && "Milim selects matching enabled skills for each request."}
                          {skillMode === "custom" && `${enabledSkills.length} pinned to this agent.`}
                          {skillMode === "none" && "No skill instructions are added for this agent."}
                        </span>
                      </div>
                    </div>
                    <div className="tool-mode-tabs" role="group" aria-label="Skill access">
                      {([
                        ["auto", "Auto"],
                        ["custom", "Custom"],
                        ["none", "None"],
                      ] as const).map(([value, label]) => (
                        <button
                          key={value}
                          type="button"
                          data-testid={`skill-mode-${value}`}
                          className={"tool-mode-button" + (skillMode === value ? " active" : "")}
                          onClick={() => setSkillMode(value)}
                        >
                          {label}
                        </button>
                      ))}
                    </div>
                    {skillMode === "custom" && (
                      <div className="agent-skill-list">
                        {enabledSkillList.length === 0 && <span className="sheet-hint">No enabled skills installed.</span>}
                        {enabledSkillList.map((skill) => (
                          <div className={"tool-row" + (enabledSkills.includes(skill.id) ? " selected" : "")} key={skill.id}>
                            <Checkbox
                              title={skill.description}
                              checked={enabledSkills.includes(skill.id)}
                              onChange={() => toggleSkill(skill.id)}
                            >
                              <span className="tool-name">{skill.name}</span>
                            </Checkbox>
                            {skill.description && <span className="tool-desc">{skill.description}</span>}
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                </section>

                <section className="agent-editor-section">
                  <div className="agent-section-head">
                    <h4>Tools</h4>
                    <span>{enabledToolCount} available</span>
                  </div>
                  <div className="tool-panel">
                    <div className="tool-panel-head">
                      <div>
                        <strong>Tool access</strong>
                        <span>
                          {toolMode === "all" && "Agent can use every enabled tool."}
                          {toolMode === "custom" && `${enabled.length} selected.`}
                          {toolMode === "none" && "No tool schemas are sent to the model."}
                        </span>
                      </div>
                    </div>

                    <div className="tool-mode-tabs" role="group" aria-label="Tool access">
                      {([
                        ["all", "All tools"],
                        ["custom", "Custom"],
                        ["none", "No tools"],
                      ] as const).map(([value, label]) => (
                        <button
                          key={value}
                          type="button"
                          data-testid={`tool-mode-${value}`}
                          className={"tool-mode-button" + (toolMode === value ? " active" : "")}
                          onClick={() => setToolModeValue(value)}
                        >
                          {label}
                        </button>
                      ))}
                    </div>

                    {toolMode === "all" && <span className="sheet-hint">Use this for agents that should decide when tools help.</span>}
                    {toolMode === "none" && <span className="sheet-hint">Use this for chat-only agents and models that reject tool calling.</span>}

                    {toolMode === "custom" && (
                      <>
                        {toolCapabilities.length > 0 && (
                          <div className="tool-capability-grid" aria-label="Tool capability groups">
                            {toolCapabilities.map((capability) => {
                              const complete = capability.selected === capability.total;
                              return (
                                <button
                                  className={"tool-capability-card" + (capability.selected > 0 ? " selected" : "")}
                                  data-testid={`tool-capability-${capability.group.toLowerCase()}`}
                                  key={capability.group}
                                  type="button"
                                  onClick={() => setToolGroupEnabled(capability.group, !complete)}
                                >
                                  <span className="tool-capability-top">
                                    <strong>{capability.group}</strong>
                                    <span>{capability.selected}/{capability.total}</span>
                                  </span>
                                  <span>{capability.detail}</span>
                                </button>
                              );
                            })}
                          </div>
                        )}
                        <input
                          className="css-input tool-search"
                          data-testid="tool-search"
                          value={toolSearch}
                          onChange={(e) => setToolSearch(e.target.value)}
                          placeholder="Search tools"
                        />
                        <div className="tool-groups">
                          {tools.length === 0 && <span className="sheet-hint">No tools available.</span>}
                          {groupedTools.map(({ group, tools: groupTools }) => (
                            <section className="tool-group" key={group}>
                              <h3>{group}</h3>
                              {groupTools.map((t) => (
                                <div className={"tool-row" + (enabled.includes(t.name) ? " selected" : "")} data-testid={`tool-row-${t.name}`} key={t.name}>
                                  <Checkbox
                                    title={t.description}
                                    checked={enabled.includes(t.name)}
                                    onChange={() => toggleTool(t.name)}
                                  >
                                    <span className="tool-name">{t.name}</span>
                                  </Checkbox>
                                  {t.description && <span className="tool-desc">{t.description}</span>}
                                </div>
                              ))}
                            </section>
                          ))}
                          {tools.length > 0 && groupedTools.length === 0 && <span className="sheet-hint">No tools match your search.</span>}
                        </div>
                      </>
                    )}
                  </div>
                </section>

                <section className="agent-editor-section agent-test-section">
                  <div className="agent-section-head">
                    <h4>Test</h4>
                    <span>{selectedAgent ? "Tools blocked for preview" : "Save first"}</span>
                  </div>
                  <div className="agent-test-panel">
                    <textarea
                      className="instr-input agent-test-prompt"
                      data-testid="agent-test-prompt"
                      value={testPrompt}
                      disabled={!selectedAgent || testRunning}
                      onChange={(e) => {
                        setTestPrompt(e.target.value);
                        setTestError("");
                        setTestResult("");
                      }}
                      placeholder={selectedAgent ? "Ask one short question to check the agent's behavior" : "Save this agent before running a preview"}
                    />
                    <button className="btn-ghost agent-test-button" data-testid="test-agent" type="button" disabled={!canTestAgent} onClick={testAgent}>
                      <Sparkles size={14} />
                      <span>{testRunning ? "Testing..." : "Test agent"}</span>
                    </button>
                    {testError && <span className="agent-draft-error">{testError}</span>}
                    {testResult && <div className="agent-test-result">{testResult}</div>}
                  </div>
                </section>

                <div className="agent-action-footer">
                  {selectedAgent && (
                    <button className="btn-accent" data-testid="start-agent-chat" type="button" onClick={() => startChat(selectedAgent)}>
                      <Plus size={14} />
                      <span>Start chat</span>
                    </button>
                  )}
                  {selectedAgent && (
                    <button className="btn-ghost agent-duplicate-action" data-testid="duplicate-agent" type="button" onClick={() => duplicateAgent(selectedAgent)}>
                      <Copy size={14} />
                      <span>Duplicate</span>
                    </button>
                  )}
                  {selectedAgent && (
                    <span className="agent-usage-note">
                      <Calendar size={13} />
                      {selectedUsage || "No schedules"}
                    </span>
                  )}
                  {sel !== "new" && (
                    <button className="btn-ghost danger agent-delete-action" type="button" onClick={remove}>
                      <Trash size={14} />
                      <span>{confirmDeleteId === selectedAgent?.id ? "Confirm delete" : "Delete"}</span>
                    </button>
                  )}
                  {confirmDeleteId === selectedAgent?.id && (
                    <span className="agent-delete-note" role="status">
                      {deleteNote}
                    </span>
                  )}
                  <span className="spacer" />
                  <button className="btn-accent" data-testid="save-agent" type="button" disabled={!canSave} onClick={save}>
                    Save
                  </button>
                </div>
              </div>
            ) : (
              <div className="agent-empty-state">
                <div className="agent-empty-icon" aria-hidden="true">
                  <Plus size={17} />
                </div>
                <h3>{agents.length ? "Select an agent" : "No agents yet"}</h3>
                <p>
                  {agents.length
                    ? "Choose a saved profile from the list, or create another reusable agent."
                    : "Create a reusable profile with its own system prompt, avatar, skills, and tool access."}
                </p>
                <button className="btn-accent agent-header-action" type="button" onClick={() => edit("new")}>
                  <Plus size={14} />
                  <span>New agent</span>
                </button>
              </div>
            )}
          </section>
        </div>
      </SheetDialog>
  );
}
