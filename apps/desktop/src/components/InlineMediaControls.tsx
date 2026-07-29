import type { MediaKind, MediaModelSchema, MediaSchemaControl } from "../api";
import { controlValue } from "../lib/media";
import { Image, Sparkles, Volume2 } from "./icons";
import { Select } from "./ui";

export function InlineMediaControls({
  providerName,
  model,
  kind,
  supportedKinds = ["image", "video", "music"],
  schema,
  schemaLoading,
  parameterValues,
  advanced,
  error,
  popover = false,
  onKindChange,
  onParameterChange,
  onAdvancedChange,
}: {
  providerName: string;
  model: string;
  kind: MediaKind;
  supportedKinds?: MediaKind[];
  schema: MediaModelSchema | null;
  schemaLoading: boolean;
  parameterValues: Record<string, unknown>;
  advanced: string;
  error?: string | null;
  popover?: boolean;
  onKindChange: (kind: MediaKind) => void;
  onParameterChange: (control: MediaSchemaControl, value: string | boolean) => void;
  onAdvancedChange: (value: string) => void;
}) {
  const visibleKinds = supportedKinds.length ? supportedKinds : [kind];
  const activeKind = visibleKinds.includes(kind) ? kind : visibleKinds[0];
  const kindOptions = visibleKinds.map((item) => ({
    value: item,
    label: item === "music" ? "Audio" : item[0].toUpperCase() + item.slice(1),
    leading: item === "music"
      ? <Volume2 size={13} />
      : item === "video"
        ? <Sparkles size={13} />
        : <Image size={13} />,
  }));
  const activeKindOption = kindOptions.find((option) => option.value === activeKind) ?? kindOptions[0];

  const controls = (
    <>
      <div className="inline-media-row">
        <div className="inline-media-kind" title={`Media type · ${providerName} / ${model}`}>
          <Select
            value={activeKind}
            options={kindOptions}
            ariaLabel="Media type"
            testId="inline-media-kind-select"
            onChange={(value) => onKindChange(value as MediaKind)}
          />
        </div>

        <div className="inline-media-parameter-controls" data-testid="inline-media-parameter-controls">
          {schemaLoading ? (
            <span className="sheet-hint">Loading parameters...</span>
          ) : (
            schema?.controls.map((control) => (
              <div
                className={"inline-media-field" + (control.kind === "array" || control.kind === "json" ? " wide" : "")}
                key={control.key}
              >
                <span title={control.description}>{control.label}</span>
                {control.kind === "select" ? (
                  <Select
                    testId={`inline-media-param-${control.key}`}
                    ariaLabel={control.label}
                    value={controlValue(parameterValues[control.key])}
                    options={(control.options ?? []).map((option) => ({
                      value: String(option.value),
                      label: option.label,
                    }))}
                    onChange={(value) => onParameterChange(control, value)}
                  />
                ) : control.kind === "checkbox" ? (
                  <input
                    className="media-checkbox-input"
                    data-testid={`inline-media-param-${control.key}`}
                    type="checkbox"
                    aria-label={control.label}
                    checked={Boolean(parameterValues[control.key])}
                    onChange={(e) => onParameterChange(control, e.target.checked)}
                  />
                ) : control.kind === "array" || control.kind === "json" ? (
                  <textarea
                    className="css-input"
                    data-testid={`inline-media-param-${control.key}`}
                    placeholder={control.placeholder}
                    aria-label={control.label}
                    value={controlValue(parameterValues[control.key])}
                    onChange={(e) => onParameterChange(control, e.target.value)}
                  />
                ) : (
                  <input
                    className="css-input"
                    data-testid={`inline-media-param-${control.key}`}
                    type={control.kind === "url" ? "url" : control.kind === "text" ? "text" : "number"}
                    placeholder={control.placeholder}
                    aria-label={control.label}
                    min={control.min}
                    max={control.max}
                    step={control.step}
                    value={controlValue(parameterValues[control.key])}
                    onChange={(e) => onParameterChange(control, e.target.value)}
                  />
                )}
              </div>
            ))
          )}
        </div>

        <details className="inline-media-advanced">
          <summary>Advanced</summary>
          <textarea
            className="css-input"
            data-testid="inline-media-advanced-input"
            aria-label="Advanced media input JSON"
            value={advanced}
            spellCheck={false}
            onChange={(e) => onAdvancedChange(e.target.value)}
          />
        </details>
      </div>

      {error && <div className="artifact-error" data-testid="inline-media-error" role="alert">{error}</div>}
    </>
  );

  return (
    <div className={`inline-media${popover ? " popover" : ""}`} data-testid="inline-media-generator" title={`${providerName} / ${model}`}>
      {popover ? (
        <details className="inline-media-disclosure">
          <summary
            className="chip inline-media-summary"
            data-testid="inline-media-settings-summary"
            aria-label={`${activeKindOption.label} settings${error ? ", error" : ""}`}
            title={error || `${activeKindOption.label} settings`}
          >
            {activeKindOption.leading}
            <span className="chip-label">{activeKindOption.label} settings</span>
            {error && <span className="dot dot-red" aria-hidden="true" />}
          </summary>
          <div className="inline-media-popover" role="group" aria-label={`${activeKindOption.label} generation settings`}>
            {controls}
          </div>
        </details>
      ) : controls}
    </div>
  );
}
