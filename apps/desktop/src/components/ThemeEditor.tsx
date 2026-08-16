import { useState } from "react";
import { themeContrastIssues } from "../theme/contrast";
import { useTheme } from "../theme/store";
import type { Theme } from "../theme/types";
import { SheetDialog } from "./SheetDialog";
import { ColorField, Select, Slider as Range, Toggle } from "./ui";

const COLOR_GROUPS: Array<{ title: string; fields: Array<[string, keyof Theme["colors"]]> }> = [
  {
    title: "Base",
    fields: [
      ["Background", "bgPrimary"],
      ["Surface", "bgSecondary"],
      ["Elevated", "bgTertiary"],
      ["Card", "cardBg"],
    ],
  },
  {
    title: "Text",
    fields: [
      ["Primary", "primaryText"],
      ["Muted", "secondaryText"],
      ["Accent light", "accentLight"],
    ],
  },
  {
    title: "Controls",
    fields: [
      ["Accent", "accent"],
      ["Border", "borderPrimary"],
      ["Input", "inputBg"],
    ],
  },
];

const FONT_PRESETS: Array<[string, string]> = [
  ["System", '-apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, "Segoe UI", Roboto, sans-serif'],
  ["Inter", "Inter, system-ui, sans-serif"],
  ["Segoe UI", '"Segoe UI", system-ui, sans-serif'],
  ["Helvetica", "Helvetica, Arial, sans-serif"],
  ["Georgia (serif)", 'Georgia, "Times New Roman", serif'],
  ["Verdana", "Verdana, Geneva, sans-serif"],
];
const MONO_PRESETS: Array<[string, string]> = [
  ["System mono", 'ui-monospace, "SF Mono", "Cascadia Code", "JetBrains Mono", monospace'],
  ["Cascadia Code", '"Cascadia Code", "Cascadia Mono", monospace'],
  ["Consolas", 'Consolas, "Courier New", monospace'],
  ["JetBrains Mono", '"JetBrains Mono", monospace'],
  ["Courier", '"Courier New", Courier, monospace'],
];

function newId() {
  return "custom-" + crypto.randomUUID();
}

function alphaColor(color: string, alpha: number): string {
  const raw = color.trim().replace(/^#/, "");
  const hex =
    raw.length === 3
      ? raw
          .split("")
          .map((char) => char + char)
          .join("")
      : raw;
  if (!/^[0-9a-f]{6}$/i.test(hex)) return color;
  const red = Number.parseInt(hex.slice(0, 2), 16);
  const green = Number.parseInt(hex.slice(2, 4), 16);
  const blue = Number.parseInt(hex.slice(4, 6), 16);
  return `rgba(${red}, ${green}, ${blue}, ${Math.max(0, Math.min(1, alpha)).toFixed(2)})`;
}

export function ThemeEditor({ base, isNew, onClose }: { base: Theme; isNew: boolean; onClose: () => void }) {
  const { saveCustom, deleteCustom, revert, preview } = useTheme();
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [draft, setDraft] = useState<Theme>(() => {
    const d = structuredClone(base);
    if (isNew) {
      d.id = newId();
      d.name = "My " + base.name;
    }
    return d;
  });
  const contrastIssues = themeContrastIssues(draft);

  function patch(mutate: (d: Theme) => void) {
    setConfirmDelete(false);
    setDraft((prev) => {
      const d = structuredClone(prev);
      mutate(d);
      preview(d);
      return d;
    });
  }

  function onUpload(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () =>
      patch((d) => {
        d.background.image = `url(${reader.result as string})`;
        d.background.imageOpacity = 1;
        d.glass.enabled = true;
      });
    reader.readAsDataURL(file);
  }

  function deleteDraft() {
    if (confirmDelete) {
      deleteCustom(draft.id);
      onClose();
      return;
    }
    setConfirmDelete(true);
  }

  return (
    <SheetDialog
      title={isNew ? "Customize theme" : "Edit theme"}
      className="sheet editor"
      testId="theme-editor"
      onClose={() => { revert(); onClose(); }}
    >
      <div className="sheet-header editor-header">
        <div className="editor-title">
          <span className="editor-kicker">{isNew ? "New theme" : "Custom theme"}</span>
          <input
            className="name-input"
            value={draft.name}
            onChange={(e) => patch((d) => { d.name = e.target.value; })}
            placeholder="Theme name"
            aria-label="Theme name"
          />
        </div>
        <div className="editor-actions">
          {!isNew && (
            <button
              className="btn-ghost danger"
              type="button"
              title={confirmDelete ? "Click again to delete this theme" : "Delete theme"}
              aria-label={confirmDelete ? `Confirm delete ${draft.name}` : `Delete ${draft.name}`}
              onClick={deleteDraft}
            >
              {confirmDelete ? "Confirm delete" : "Delete"}
            </button>
          )}
          <button className="btn-ghost" type="button" onClick={() => { revert(); onClose(); }}>Cancel</button>
          <button className="btn-accent" type="button" disabled={contrastIssues.length > 0} onClick={() => { saveCustom(draft); onClose(); }}>Save</button>
        </div>
      </div>

      <div className="editor-body">
        <div className="editor-main">
          <section className="editor-section editor-section-colors">
            <div className="editor-section-head">
              <h3>Colors</h3>
              <span>{draft.isDark ? "Dark" : "Light"}</span>
            </div>
            <div className="editor-color-groups">
              {COLOR_GROUPS.map((group) => (
                <div className="editor-color-group" key={group.title}>
                  <div className="editor-color-group-title">{group.title}</div>
                  <div className="color-grid">
                    {group.fields.map(([label, key]) => (
                      <ColorField
                        key={key}
                        label={(
                          <span className="editor-color-label">
                            <span>{label}</span>
                            <code>{draft.colors[key]}</code>
                          </span>
                        )}
                        value={draft.colors[key]}
                        onChange={(v) => patch((d) => { d.colors[key] = v; })}
                      />
                    ))}
                  </div>
                </div>
              ))}
            </div>
            <div className="editor-theme-mode-row">
              <div>
                <strong>Contrast mode</strong>
                <span>Changes how accent contrast is validated.</span>
              </div>
              <Toggle
                checked={draft.isDark}
                onChange={(v) => patch((d) => { d.isDark = v; })}
                ariaLabel="Dark theme contrast mode"
              />
            </div>
            {contrastIssues.length > 0 && (
              <p className="sheet-hint error">{contrastIssues[0]}</p>
            )}
          </section>

          <section className="editor-section editor-section-background">
            <div className="editor-section-head">
              <h3>Background</h3>
              <span>{draft.background.image ? "Image" : "Solid"}</span>
            </div>
            <div className="editor-background-actions">
              <label className="btn-file">
                Upload image
                <input type="file" accept="image/*" onChange={onUpload} hidden />
              </label>
              {draft.background.image && (
                <button className="btn-ghost" type="button" onClick={() => patch((d) => { d.background.image = undefined; })}>
                  Clear image
                </button>
              )}
            </div>
            <label className="text-row editor-css-field">
              <span>Image or gradient CSS</span>
              <input
                className="css-input"
                value={draft.background.image ?? ""}
                placeholder="url(...) or linear-gradient(...)"
                onChange={(e) => patch((d) => { d.background.image = e.target.value || undefined; })}
              />
            </label>
            <div className="editor-background-controls">
              <Slider label="Image blur" min={0} max={40} step={1} value={draft.background.imageBlur ?? 0}
                onChange={(v) => patch((d) => { d.background.imageBlur = v; })} />
              <label className="editor-inline-color">
                <span>Overlay tint</span>
                <ColorField
                  value={draft.background.overlayColor ?? "#000000"}
                  onChange={(v) => patch((d) => { d.background.overlayColor = v; })}
                />
              </label>
              <Slider label="Overlay opacity" min={0} max={0.85} step={0.02} value={draft.background.overlayOpacity}
                onChange={(v) => patch((d) => { d.background.overlayOpacity = v; })} />
            </div>
          </section>

          <section className="editor-section editor-typography">
            <div className="editor-section-head">
              <h3>Typography</h3>
            </div>
            <div className="editor-type-controls">
              <label className="text-row">
                <span>Interface font</span>
                <Select
                  value={draft.typography.fontFamily}
                  options={FONT_PRESETS.map(([label, val]) => ({ label, value: val }))}
                  onChange={(v) => patch((d) => { d.typography.fontFamily = v; })}
                />
              </label>
              <label className="text-row">
                <span>Code font</span>
                <Select
                  value={draft.typography.monoFamily}
                  options={MONO_PRESETS.map(([label, val]) => ({ label, value: val }))}
                  onChange={(v) => patch((d) => { d.typography.monoFamily = v; })}
                />
              </label>
            </div>
            <div className="editor-type-sample" style={{ fontFamily: draft.typography.fontFamily }}>
              <span>Thread title sample</span>
              <strong>Design pass for settings</strong>
              <code style={{ fontFamily: draft.typography.monoFamily }}>pnpm -C apps/desktop build</code>
            </div>
          </section>
        </div>

        <aside className="editor-side">
          <section className="editor-preview-card" aria-label="Theme preview">
            <div className="editor-section-head">
              <h3>Preview</h3>
              <span>{draft.name}</span>
            </div>
            <div
              className="theme-editor-preview"
              style={{
                background: draft.background.image
                  ? `${draft.background.image}, ${draft.colors.bgPrimary}`
                  : draft.colors.bgPrimary,
                color: draft.colors.primaryText,
                borderColor: draft.colors.borderPrimary,
              }}
            >
              <div className="theme-editor-preview-overlay" style={{
                background: draft.background.overlayColor ?? "#000000",
                opacity: draft.background.overlayOpacity,
              }} />
              <div
                className="theme-editor-preview-sidebar"
                style={{
                  background: alphaColor(draft.colors.bgPrimary, draft.glass.enabled ? draft.glass.opacitySecondary : 1),
                  borderColor: draft.colors.borderPrimary,
                }}
              >
                <span style={{ background: draft.colors.accent }} />
                <span style={{ background: draft.colors.secondaryText }} />
                <span style={{ background: draft.colors.tertiaryText }} />
              </div>
              <div
                className="theme-editor-preview-panel"
                style={{
                  background: alphaColor(draft.colors.bgSecondary, draft.glass.enabled ? draft.glass.opacityPrimary : 1),
                  borderColor: draft.colors.borderPrimary,
                  borderRadius: draft.borders.cardRadius,
                  backdropFilter: draft.glass.enabled ? `blur(${draft.glass.blurRadius}px)` : undefined,
                }}
              >
                <div className="theme-editor-preview-topline">
                  <span style={{ background: draft.colors.accent }} />
                  <span style={{ background: draft.colors.tertiaryText }} />
                </div>
                <div className="theme-editor-preview-message" style={{ background: draft.colors.bgTertiary }} />
                <div
                  className="theme-editor-preview-message compact"
                  style={{ background: draft.colors.cardBg, borderColor: draft.colors.cardBorder }}
                />
                <div
                  className="theme-editor-preview-input"
                  style={{
                    background: draft.colors.inputBg,
                    borderColor: draft.colors.inputBorder,
                    borderRadius: draft.borders.inputRadius,
                  }}
                />
              </div>
            </div>
            <div className="editor-swatch-row" aria-label="Theme color summary">
              {[
                ["Base", draft.colors.bgPrimary],
                ["Panel", draft.colors.bgSecondary],
                ["Text", draft.colors.primaryText],
                ["Accent", draft.colors.accent],
              ].map(([label, color]) => (
                <span className="editor-swatch" key={`${label}-${color}`} title={color}>
                  <span style={{ background: color }} />
                  <small>{label}</small>
                </span>
              ))}
            </div>
          </section>

          <section className="editor-section compact">
            <div className="editor-section-head">
              <h3>Glass</h3>
              <span>{draft.glass.enabled ? "On" : "Off"}</span>
            </div>
            <Toggle
              checked={draft.glass.enabled}
              onChange={(v) => patch((d) => { d.glass.enabled = v; })}
              label="Translucent, blurred panels"
            />
            <Slider label="Blur" min={0} max={40} step={1} value={draft.glass.blurRadius} suffix="px"
              onChange={(v) => patch((d) => { d.glass.blurRadius = v; })} />
            <Slider label="Panel opacity" min={0.2} max={1} step={0.02} value={draft.glass.opacityPrimary}
              onChange={(v) => patch((d) => { d.glass.opacityPrimary = v; d.glass.opacitySecondary = Math.max(0.2, v - 0.15); })} />
          </section>

          <section className="editor-section compact">
            <div className="editor-section-head">
              <h3>Shape</h3>
              <span>{draft.borders.cardRadius}px cards</span>
            </div>
            <Slider label="Card radius" min={0} max={24} step={1} value={draft.borders.cardRadius} suffix="px"
              onChange={(v) => patch((d) => { d.borders.cardRadius = v; })} />
            <Slider label="Input radius" min={0} max={28} step={1} value={draft.borders.inputRadius} suffix="px"
              onChange={(v) => patch((d) => { d.borders.inputRadius = v; })} />
          </section>
        </aside>
      </div>
    </SheetDialog>
  );
}

function Slider({
  label,
  min,
  max,
  step,
  value,
  suffix = "",
  onChange,
}: {
  label: string;
  min: number;
  max: number;
  step: number;
  value: number;
  suffix?: string;
  onChange: (v: number) => void;
}) {
  return (
    <div className="slider-row">
      <span className="slider-label">{label}</span>
      <Range min={min} max={max} step={step} value={value} onChange={onChange} />
      <span className="slider-val">{suffix === "px" ? Math.round(value) : value.toFixed(2)}{suffix}</span>
    </div>
  );
}
