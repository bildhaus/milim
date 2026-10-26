import { useSettings } from "./store";
import { useEffect, useState } from "react";
import { SettingsBlock, FieldIssue } from "./SettingsPrimitives";
import { WORKER_LIMIT_DEFAULTS } from "../lib/runLimits";

function RunLimitInput({ id, value, label, placeholder, max, fractional, onChange }: {
  id: string; value: number | null; label: string; placeholder: string; max: number;
  fractional: boolean; onChange: (value: number | null) => void;
}) {
  const [draft, setDraft] = useState(value == null ? "" : String(value));
  const [issue, setIssue] = useState<string | null>(null);
  useEffect(() => { setDraft(value == null ? "" : String(value)); setIssue(null); }, [value]);
  return <div className="setting-field">
    <label className="setting-mini-title" htmlFor={id}>{label}</label>
    <input id={id} data-testid={id} type="number" min={fractional ? "0.000001" : "1"}
      max={max} step={fractional ? "any" : "1"} value={draft} placeholder={placeholder}
      aria-invalid={Boolean(issue)} aria-describedby={issue ? `${id}-issue` : undefined}
      onChange={(event) => { setDraft(event.currentTarget.value); setIssue(null); }}
      onBlur={(event) => {
        if (!event.currentTarget.validity.valid) { setIssue(`Enter a positive ${fractional ? "amount" : "whole number"} up to ${max.toLocaleString()}.`); return; }
        setIssue(null);
        onChange(draft === "" ? null : Number(draft));
      }} />
    {issue && <div id={`${id}-issue`}><FieldIssue message={issue} /></div>}
  </div>;
}

export function RunLimitsSettings() {
  const limits = useSettings((state) => state.runLimits);
  const setLimits = useSettings((state) => state.setRunLimits);
  return (
    <SettingsBlock title="Provider run limits" data-setting-id="models-run-limits">
      <div className="setting-stack">
        <p className="sheet-hint">Applied to each newly accepted provider run, including mobile sends. Existing runs keep their limits. Codex, Claude, OpenCode, and Pi manage their own execution.</p>
        {([
          ["maxSteps", "Model steps", "100", 10_000],
          ["maxSeconds", "Time in seconds", "No time limit", 86_400],
          ["maxCostUsd", "Spend threshold in USD", "No spend threshold", 1_000_000],
        ] as const).map(([key, label, placeholder, max]) => (
          <RunLimitInput key={key} id={`run-limit-${key}`} label={label} placeholder={placeholder} max={max}
            fractional={key === "maxCostUsd"} value={limits[key]} onChange={(value) => setLimits({ ...limits, [key]: value })} />
        ))}
        <p className="sheet-hint">Limits are checked between model and tool steps. An in-flight step can exceed the time or spend threshold. Cost uses reported billing or a cached estimate; missing cost pauses further steps. Send Continue to start another bounded run. Queued prompts stay paused until you resume them.</p>
      </div>
    </SettingsBlock>
  );
}

export function WorkerLimitsSettings() {
  const limits = useSettings((state) => state.workerLimits);
  const setLimits = useSettings((state) => state.setWorkerLimits);
  return (
    <SettingsBlock title="Worker budget" data-setting-id="models-worker-limits">
      <div className="setting-stack">
        <p className="sheet-hint">Applied to each delegated provider Worker when its Worker Run starts. A Worker that uses up its steps or tokens stops between steps; its report keeps what it wrote so far.</p>
        {([
          ["maxSteps", "Model steps per Worker", WORKER_LIMIT_DEFAULTS.maxSteps, 1_000],
          ["maxTokens", "Tokens per Worker", WORKER_LIMIT_DEFAULTS.maxTokens, 100_000_000],
          ["maxMinutes", "Worker Run time limit in minutes", WORKER_LIMIT_DEFAULTS.maxMinutes, 1_440],
        ] as const).map(([key, label, fallback, max]) => (
          <RunLimitInput key={key} id={`worker-limit-${key}`} label={label} placeholder={fallback.toLocaleString()} max={max}
            fractional={false} value={limits[key]} onChange={(value) => setLimits({ ...limits, [key]: value })} />
        ))}
        <p className="sheet-hint">Tokens count prompt and output tokens across a Worker's steps. The time limit is a safety net for the whole run, including account-runtime Workers; leave a field empty for its default.</p>
      </div>
    </SettingsBlock>
  );
}
