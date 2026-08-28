import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import {
  type ModelInfo,
  type PrivacyMode,
  type ProviderInfo,
  type ReasoningEffort,
  type RunTrace,
  type ToolApprovalMode,
} from "../api";
import { goalChipVisible, type GoalSettings } from "../lib/goals";
import { normalizeGenerationSettings, type GenerationSettings } from "../lib/generationSettings";
import { modelDevProfile, modelDisplayName } from "../lib/modelPicker";
import { REASONING_EFFORT_LABEL, reasoningEffortForThread } from "../lib/reasoningEffort";
import { ChevronDown, Cube, Lightbulb, Pin, Sliders } from "./icons";
import { ModelPicker, type ModelPickerSelection } from "./ModelPicker";
import { ProviderIcon, providerBrandForModel } from "./ProviderIcon";
import { RunTimeline } from "./RunTimeline";

function Shield({ size = 13 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.7}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M12 3l7 3v5c0 4.4-3 7.6-7 9-4-1.4-7-4.6-7-9V6l7-3z" />
    </svg>
  );
}

const PRIVACY_LABEL: Record<PrivacyMode, string> = {
  off: "Off",
  redact: "Redact",
  block: "Block",
};

const TOOL_APPROVAL_LABEL: Record<ToolApprovalMode, string> = {
  review: "Review",
  guarded: "Guarded",
  open: "Open",
};

const TOOL_APPROVAL_DESCRIPTION: Record<ToolApprovalMode, string> = {
  review: "Run read-only tools; ask before consequential actions.",
  guarded: "Read-only tools only; consequential actions are unavailable.",
  open: "Run without approval in trusted workspaces.",
};

export function modelPickerPlacement(
  triggerTop: number,
  triggerBottom: number,
  viewportHeight: number,
): CSSProperties {
  const edge = 8;
  const gap = 6;
  const above = Math.max(0, triggerTop - edge - gap);
  const below = Math.max(0, viewportHeight - triggerBottom - edge - gap);
  const openBelow = above < 160 && below > above;
  return {
    top: openBelow ? `calc(100% + ${gap}px)` : "auto",
    bottom: openBelow ? "auto" : `calc(100% + ${gap}px)`,
    maxHeight: Math.min(440, openBelow ? below : above),
  };
}

function Monitor({ size = 13 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.7}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <rect x="3" y="4" width="18" height="12" rx="1.5" />
      <path d="M8 20h8M12 16v4" />
    </svg>
  );
}

export function ControlBar({
  models,
  model,
  reasoningEffortByModel,
  reasoningEffortOverrides,
  onReasoningEffort,
  generationSettings,
  onGenerationSettings,
  providers,
  toolIntent,
  onModel,
  sandbox,
  onToggleSandbox,
  computerUse,
  onToggleComputer,
  memory,
  onToggleMemory,
  planMode,
  onTogglePlanMode,
  privacy,
  onPrivacy,
  toolApproval,
  onToolApproval,
  onManageProviders,
  onManageMcp,
  onManageMemory,
  goal,
  goalMode,
  onToggleGoalMode,
  onOpenGoal,
  activeRun,
  inlineControls,
}: {
  models: ModelInfo[];
  model: string;
  reasoningEffortByModel: Record<string, ReasoningEffort>;
  reasoningEffortOverrides?: Record<string, ReasoningEffort>;
  onReasoningEffort?: (modelId: string, effort: ReasoningEffort) => void;
  generationSettings?: GenerationSettings;
  onGenerationSettings?: (settings: GenerationSettings) => void;
  providers?: ProviderInfo[];
  toolIntent?: boolean;
  onModel: (selection: ModelPickerSelection) => void;
  sandbox: boolean;
  onToggleSandbox: () => void;
  computerUse: boolean;
  onToggleComputer: () => void;
  memory: boolean;
  onToggleMemory: () => void;
  planMode: boolean;
  onTogglePlanMode: () => void;
  privacy: PrivacyMode;
  onPrivacy: (privacy: PrivacyMode) => void;
  toolApproval: ToolApprovalMode;
  onToolApproval: (approval: ToolApprovalMode) => void;
  onManageProviders: () => void;
  onManageMcp: () => void;
  onManageMemory: () => void;
  goal: GoalSettings;
  goalMode?: boolean;
  onToggleGoalMode?: () => void;
  onOpenGoal: () => void;
  activeRun?: RunTrace | null;
  inlineControls?: ReactNode;
}) {
  const [menu, setMenu] = useState<null | "model" | "context">(null);
  const [modelPickerStyle, setModelPickerStyle] = useState<CSSProperties>();
  const ref = useRef<HTMLDivElement>(null);
  const modelTriggerRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!menu) return;
    const onDoc = (e: MouseEvent) => {
      const target = e.target;
      if (target instanceof Element && target.closest(".mp-effort-menu"))
        return;
      if (
        ref.current &&
        target instanceof Node &&
        !ref.current.contains(target)
      )
        setMenu(null);
    };
    const closeOnResize = () => setMenu(null);
    document.addEventListener("mousedown", onDoc);
    window.addEventListener("resize", closeOnResize);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      window.removeEventListener("resize", closeOnResize);
    };
  }, [menu]);

  const contextAccessibleLabel = `Session controls, Docker sandbox ${sandbox ? "on" : "off"}, Computer ${computerUse ? "on" : "off"}, Memory ${memory ? "on" : "off"}, Privacy ${PRIVACY_LABEL[privacy]}, Tool approval ${TOOL_APPROVAL_LABEL[toolApproval]}`;
  const showGoalChip = Boolean(goalMode) || goalChipVisible(goal);
  const goalDetail = goalMode
    ? "Ready"
    : goal.status[0].toUpperCase() + goal.status.slice(1);
  const activeModel = models.find((item) => item.id === model);
  const activeModelLabel = activeModel ? modelDisplayName(activeModel) : model;
  const activeReasoningEffort = reasoningEffortForThread(reasoningEffortOverrides, reasoningEffortByModel, model, models);
  const activeReasoningEffortLabel = activeReasoningEffort === "auto" ? "" : REASONING_EFFORT_LABEL[activeReasoningEffort];
  const activeModelProfile = modelDevProfile(activeModel, model, {
    providers,
    toolIntent,
    planMode,
  });
  const activeModelRoute = [
    activeModelProfile.providerLabel,
    activeModelProfile.laneLabel,
  ].filter(Boolean).join(" / ");
  const activeModelDot =
    activeModelProfile.setupTone === "error"
      ? "dot-red"
      : activeModelProfile.setupTone === "warning"
        ? "dot-yellow"
        : activeModelProfile.setupTone === "off"
          ? "dot-off"
          : "dot-green";
  const activeProviderBrand = providerBrandForModel(activeModel, providers);
  const generationOverrideCount = Object.keys(generationSettings ?? {}).length;
  const setGenerationNumber = (key: keyof GenerationSettings, raw: string) => {
    if (!onGenerationSettings) return false;
    const next = { ...(generationSettings ?? {}) };
    if (!raw.trim()) delete next[key];
    else (next as Record<string, unknown>)[key] = Number(raw);
    const normalized = normalizeGenerationSettings(next);
    onGenerationSettings(normalized);
    return !raw.trim() || normalized[key] !== undefined;
  };

  return (
    <div className="control-bar">
      <div className="chips" ref={ref}>
        {/* Model */}
        <div className="chip-wrap">
          <button
            ref={modelTriggerRef}
            type="button"
            className="chip chip-model"
            data-testid="model-picker-trigger"
            onClick={(event) => {
              if (menu === "model") {
                setMenu(null);
                return;
              }
              const rect = event.currentTarget.getBoundingClientRect();
              setModelPickerStyle(modelPickerPlacement(rect.top, rect.bottom, window.innerHeight));
              setMenu("model");
            }}
            title={`${activeModelProfile.routeDetail} ${activeModelProfile.setupDetail}`}
            aria-label={`Choose model${activeModelLabel ? `, current model ${activeModelLabel}` : ""}${activeReasoningEffortLabel ? `, reasoning effort ${activeReasoningEffortLabel}` : ""}, ${activeModelRoute || activeModelProfile.setupLabel}`}
            aria-haspopup="dialog"
            aria-expanded={menu === "model"}
          >
            <span className={`dot ${activeModelDot}`} />
            <ProviderIcon brand={providerBrandForModel(activeModel, providers)} />
            <span className="chip-label">{activeModelLabel || "Choose model"}</span>
            {(activeReasoningEffortLabel || activeModelProfile.laneLabel) && (
              <span className="chip-detail">
                {activeReasoningEffortLabel || activeModelProfile.laneLabel}
              </span>
            )}
            <ChevronDown size={12} className="chip-chev" />
          </button>
          {menu === "model" && (
            <ModelPicker
              models={models}
              model={model}
              providers={providers}
              toolIntent={toolIntent}
              planMode={planMode}
              onModel={onModel}
              onManageProviders={onManageProviders}
              onManageMcp={onManageMcp}
              onManageMemory={onManageMemory}
              reasoningEffortOverrides={reasoningEffortOverrides}
              onReasoningEffort={onReasoningEffort}
              onClose={() => {
                setMenu(null);
                window.requestAnimationFrame(() => modelTriggerRef.current?.focus());
              }}
              style={modelPickerStyle}
            />
          )}
        </div>

        {inlineControls && (
          <div className="control-inline-slot">{inlineControls}</div>
        )}

        {showGoalChip && (
          <button
            type="button"
            className="chip chip-on"
            data-testid={goalMode ? "goal-mode-chip" : "goal-panel-trigger"}
            onClick={goalMode ? onToggleGoalMode : onOpenGoal}
            title={
              goalMode
                ? "Goal mode is active. Your next prompt becomes the goal. Click to turn it off."
                : "Goal"
            }
            aria-label={
              goalMode
                ? "Goal mode active, next prompt becomes the goal"
                : `Goal, ${goalDetail}`
            }
          >
            <Pin size={13} />
            <span className="chip-label">Goal</span>
            <span className="chip-detail">{goalDetail}</span>
          </button>
        )}

        {planMode && (
          <button
            type="button"
            className="chip chip-on"
            data-testid="plan-mode-chip"
            onClick={onTogglePlanMode}
            title="Plan Mode is active. Click to turn it off."
            aria-label="Plan Mode active, read-only"
          >
            <Lightbulb size={13} />
            <span className="chip-label">Plan</span>
            <span className="chip-detail">Read-only</span>
          </button>
        )}

        <div className="context-cluster">
          {activeRun && (
            <div className="control-run-wrap">
              <RunTimeline run={activeRun} />
            </div>
          )}

          {/* Session controls */}
          <div className="chip-wrap context-chip-wrap">
            <button
              type="button"
              className={
                "chip context-chip" +
                (toolApproval === "open" ? " chip-on" : "")
              }
              data-testid="context-menu-trigger"
              onClick={() =>
                setMenu((m) => (m === "context" ? null : "context"))
              }
              title={`Privacy ${PRIVACY_LABEL[privacy]}, approval ${TOOL_APPROVAL_LABEL[toolApproval]}`}
              aria-label={contextAccessibleLabel}
              aria-haspopup="dialog"
              aria-expanded={menu === "context"}
            >
              <Sliders size={13} />
              <span className="chip-label">{TOOL_APPROVAL_LABEL[toolApproval]}</span>
              <ChevronDown size={12} className="chip-chev" />
            </button>
            {menu === "context" && (
              <div
                className="context-menu"
                role="dialog"
                aria-label="Session controls"
              >
                <button
                  className={"context-row" + (sandbox ? " context-on" : "")}
                  type="button"
                  onClick={onToggleSandbox}
                  aria-pressed={sandbox}
                  title="Run tools in an isolated Docker sandbox"
                >
                  <span className="context-icon">
                    <Cube size={14} />
                  </span>
                  <span className="context-title">Docker sandbox</span>
                  <span className="context-switch" aria-hidden="true" />
                </button>

                <button
                  className={
                    "context-row" + (computerUse ? " context-on" : "")
                  }
                  type="button"
                  onClick={onToggleComputer}
                  aria-pressed={computerUse}
                  title="Let the agent see the screen and control the mouse/keyboard"
                >
                  <span className="context-icon">
                    <Monitor size={14} />
                  </span>
                  <span className="context-title">Computer use</span>
                  <span className="context-switch" aria-hidden="true" />
                </button>

                <div className={"context-row context-memory-row" + (memory ? " context-on" : "")}>
                  <span className="context-icon">
                    <Lightbulb size={14} />
                  </span>
                  <span className="context-title">Memory</span>
                  <span className="context-memory-actions">
                    <button
                      className="context-manage-button"
                      type="button"
                      aria-label="Manage memory"
                      onClick={() => {
                        setMenu(null);
                        onManageMemory();
                      }}
                      title="Manage personal and project memory"
                    >
                      Manage
                      <ChevronDown size={12} className="context-manage-chev" />
                    </button>
                    <button
                      className="context-toggle-button"
                      type="button"
                      data-testid="memory-toggle"
                      onClick={onToggleMemory}
                      aria-label="Toggle memory"
                      aria-pressed={memory}
                      title="Let the agent use personal and project memories"
                    >
                      <span className="context-switch" aria-hidden="true" />
                    </button>
                  </span>
                </div>

                <div className={"context-row context-choice-row" + (privacy !== "off" ? " context-on" : "")}>
                  <span className="context-icon">
                    <Shield size={14} />
                  </span>
                  <span className="context-title">Privacy</span>
                  <span className="context-choice-group" role="radiogroup" aria-label="Privacy">
                    {(["off", "redact", "block"] as const).map((value) => (
                      <button
                        key={value}
                        type="button"
                        role="radio"
                        aria-checked={privacy === value}
                        className={privacy === value ? "active" : ""}
                        title={value === "off" ? "Send without PII scanning" : value === "redact" ? "Redact detected PII before remote sends" : "Block remote sends when PII is detected"}
                        onClick={() => onPrivacy(value)}
                      >
                        {PRIVACY_LABEL[value]}
                      </button>
                    ))}
                  </span>
                </div>

                <div className="context-row context-choice-row">
                  <span className="context-icon">
                    <Shield size={14} />
                  </span>
                  <span className="context-title">Tool approval</span>
                  <span
                    className="context-choice-group"
                    role="radiogroup"
                    aria-label="Tool approval"
                    aria-describedby="tool-approval-description"
                  >
                    {(["review", "guarded", "open"] as const).map((value) => (
                      <button
                        key={value}
                        type="button"
                        role="radio"
                        aria-checked={toolApproval === value}
                        className={toolApproval === value ? "active" : ""}
                        title={TOOL_APPROVAL_DESCRIPTION[value]}
                        onClick={() => onToolApproval(value)}
                      >
                        {TOOL_APPROVAL_LABEL[value]}
                      </button>
                    ))}
                  </span>
                  <span
                    id="tool-approval-description"
                    className="context-choice-description"
                  >
                    {TOOL_APPROVAL_DESCRIPTION[toolApproval]}
                  </span>
                </div>

                {model && onGenerationSettings && !["codex", "claude", "opencode", "pi"].includes(activeProviderBrand ?? "") && (
                  <details className="generation-controls">
                    <summary>
                      <span>Generation</span>
                      <span>{generationOverrideCount ? `${generationOverrideCount} customized` : "Model defaults"}</span>
                    </summary>
                    <p>Overrides for {activeModelLabel || model}. Blank fields use the server default.</p>
                    <div className="generation-grid">
                      {([
                        ["maxTokens", "Output tokens", "1–1,000,000", "1"],
                        ["temperature", "Temperature", "0–2", "0.01"],
                        ["topP", "Top P", ">0–1", "0.01"],
                        ["seed", "Seed", "integer", "1"],
                        ["frequencyPenalty", "Frequency penalty", "−2–2", "0.01"],
                        ["presencePenalty", "Presence penalty", "−2–2", "0.01"],
                      ] as const).map(([key, label, placeholder, step]) => (
                        <label key={key}>
                          <span>{label}</span>
                          <input
                            key={`${model}-${key}-${generationSettings?.[key] ?? "default"}`}
                            type="number"
                            defaultValue={generationSettings?.[key] ?? ""}
                            placeholder={placeholder}
                            step={step}
                            onBlur={(event) => {
                              if (!setGenerationNumber(key, event.currentTarget.value)) event.currentTarget.value = "";
                            }}
                          />
                        </label>
                      ))}
                      {activeProviderBrand === "vllm" && ([
                        ["topK", "Top K", "−1 or ≥1", "1"],
                        ["minP", "Min P", "0–1", "0.01"],
                        ["repetitionPenalty", "Repetition penalty", ">0–2", "0.01"],
                        ["thinkingTokenBudget", "Thinking budget", "0–1,000,000", "1"],
                      ] as const).map(([key, label, placeholder, step]) => (
                        <label key={key}>
                          <span>{label}<em>vLLM</em></span>
                          <input
                            key={`${model}-${key}-${generationSettings?.[key] ?? "default"}`}
                            type="number"
                            defaultValue={generationSettings?.[key] ?? ""}
                            placeholder={placeholder}
                            step={step}
                            onBlur={(event) => {
                              if (!setGenerationNumber(key, event.currentTarget.value)) event.currentTarget.value = "";
                            }}
                          />
                        </label>
                      ))}
                    </div>
                    <label className="generation-stop">
                      <span>Stop sequences <small>one per line, up to 8</small></span>
                      <textarea
                        value={generationSettings?.stop?.join("\n") ?? ""}
                        placeholder="Model default"
                        rows={2}
                        onChange={(event) => onGenerationSettings({
                          ...(generationSettings ?? {}),
                          stop: event.currentTarget.value ? event.currentTarget.value.split("\n") : undefined,
                        })}
                      />
                    </label>
                    {generationOverrideCount > 0 && (
                      <button type="button" className="generation-reset" onClick={() => onGenerationSettings({})}>
                        Reset generation overrides
                      </button>
                    )}
                  </details>
                )}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
