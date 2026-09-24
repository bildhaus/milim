import { useEffect, useMemo, useState } from "react";
import { createSkill, deleteSkill, listSkills, updateSkill, type SkillInfo } from "../api";
import { Lightbulb, Plus, Search, Trash, X } from "./icons";
import { SheetDialog } from "./SheetDialog";
import { PaneResizeHandle } from "./PaneResizeHandle";
import { MANAGER_DETAIL_MIN_WIDTH } from "../lib/paneSizes";
import { useSplitPane } from "../ui/usePaneResize";
import { Toggle } from "./ui";
import "./AgentsManager.css";

type Selection = SkillInfo | "new" | null;
type CreateMode = "manual" | "paste" | "url";

function sourceLabel(skill: SkillInfo): string {
  if (skill.source_kind === "github") return "GitHub";
  if (skill.source_kind === "global") return "Global";
  if (skill.source_kind === "pasted") return "Pasted";
  return "Manual";
}

function rowMeta(skill: SkillInfo): string {
  return [
    skill.source_kind === "global" ? "" : sourceLabel(skill),
    skill.enabled ? "" : "Disabled",
  ].filter(Boolean).join(" / ");
}

function emptyDraft() {
  return {
    name: "",
    description: "",
    instructions: "",
    enabled: true,
    skillMd: "",
    skillUrl: "",
    mode: "manual" as CreateMode,
  };
}

export function SkillsManager({ onClose }: { onClose: () => void }) {
  const rail = useSplitPane("skillsRail", "--manager-rail-width", MANAGER_DETAIL_MIN_WIDTH);
  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [sel, setSel] = useState<Selection>(null);
  const [q, setQ] = useState("");
  const [draft, setDraft] = useState(emptyDraft);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  const refresh = () => listSkills().then(setSkills);
  useEffect(() => {
    refresh();
  }, []);

  const filtered = useMemo(() => {
    const needle = q.trim().toLowerCase();
    if (!needle) return skills;
    return skills.filter((s) =>
      [s.name, s.description, s.instructions, s.source_url ?? ""].some((v) => v.toLowerCase().includes(needle)),
    );
  }, [q, skills]);

  function edit(skill: SkillInfo | "new") {
    setSel(skill);
    setNote("");
    setConfirmDeleteId(null);
    if (skill === "new") {
      setDraft(emptyDraft());
    } else {
      setDraft({
        name: skill.name,
        description: skill.description,
        instructions: skill.instructions,
        enabled: skill.enabled,
        skillMd: "",
        skillUrl: skill.source_url ?? "",
        mode: "manual",
      });
    }
  }

  async function save() {
    setBusy(true);
    setNote("");
    setConfirmDeleteId(null);
    const saved = sel === "new"
      ? draft.mode === "paste"
        ? await createSkill({ skill_md: draft.skillMd, enabled: draft.enabled })
        : draft.mode === "url"
          ? await createSkill({ skill_url: draft.skillUrl, enabled: draft.enabled })
          : await createSkill({
              name: draft.name.trim(),
              description: draft.description,
              instructions: draft.instructions,
              enabled: draft.enabled,
            })
      : sel
        ? await updateSkill({
            ...sel,
            name: draft.name.trim(),
            description: draft.description,
            instructions: draft.instructions,
            enabled: draft.enabled,
          })
        : null;
    setBusy(false);
    if (!saved) {
      setNote("Could not save skill.");
      return;
    }
    await refresh();
    setSel(saved);
    setDraft((d) => ({ ...d, name: saved.name, description: saved.description, instructions: saved.instructions, enabled: saved.enabled }));
    setNote("Saved.");
  }

  async function remove() {
    if (!sel || sel === "new") return;
    if (confirmDeleteId !== sel.id) {
      setConfirmDeleteId(sel.id);
      return;
    }
    await deleteSkill(sel.id);
    await refresh();
    setSel(null);
    setConfirmDeleteId(null);
  }

  const selectedSkill = sel && sel !== "new" ? sel : null;
  const canSave = !busy && (
    sel === "new"
      ? draft.mode === "paste"
        ? draft.skillMd.trim().length > 0
        : draft.mode === "url"
          ? draft.skillUrl.trim().length > 0
          : draft.name.trim().length > 0
      : Boolean(sel) && draft.name.trim().length > 0
  );

  return (
    <SheetDialog title="Skills" className="sheet agents-sheet agent-manager-sheet skills-manager-sheet" resizable={{ id: "skills" }} onClose={onClose}>
      <div className="agent-manager-header">
        <div className="agent-manager-title">
          <h2>Skills</h2>
          <p>Reusable instructions loaded automatically when they match a chat.</p>
        </div>
        <div className="agent-manager-header-actions">
          <button className="btn-accent agent-header-action" type="button" onClick={() => edit("new")}>
            <Plus size={14} />
            <span>New skill</span>
          </button>
          <button className="icon-btn sheet-close agent-close" type="button" onClick={onClose} title="Close" aria-label="Close skills">
            <X size={15} />
          </button>
        </div>
      </div>

      <div ref={rail.containerRef} className="agent-manager-body" style={rail.style}>
        <PaneResizeHandle resize={rail.resize} className="manager-rail-resize-handle" data-testid="skills-rail-resize-handle" />
        <aside className="agent-rail" aria-label="Installed skills">
          <div className="agent-rail-summary">
            <span>{skills.filter((s) => s.enabled).length} enabled</span>
            <span>{skills.length} installed</span>
          </div>
          <div className="mp-search">
            <Search size={14} />
            <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search skills" aria-label="Search skills" />
          </div>
          <button className="agent-rail-action" type="button" onClick={() => edit("new")}>
            <Plus size={14} />
            <span>New</span>
          </button>
          {filtered.length > 0 ? (
            <div className="agent-list" role="list">
              {filtered.map((skill) => {
                const meta = rowMeta(skill);
                return (
                  <button
                    key={skill.id}
                    className={
                      "agent-list-card" +
                      (selectedSkill?.id === skill.id ? " active" : "") +
                      (!skill.enabled ? " disabled" : "")
                    }
                    type="button"
                    onClick={() => edit(skill)}
                  >
                    <span className="agent-card-copy">
                      <span className="agent-card-name">{skill.name}</span>
                      {meta && <span className="skill-card-meta">{meta}</span>}
                    </span>
                  </button>
                );
              })}
            </div>
          ) : (
            <div className="agent-list-placeholder">
              <span>{skills.length ? "No matches" : "No skills"}</span>
            </div>
          )}
        </aside>

        <section className="agent-editor-panel" aria-label="Skill editor">
          {sel ? (
            <div className="agent-editor">
              <div className="agent-editor-head">
                <div>
                  <span className="agent-editor-kicker">{sel === "new" ? "New skill" : sourceLabel(sel)}</span>
                  <h3>{sel === "new" ? "Install skill" : draft.name || selectedSkill?.name}</h3>
                </div>
                <div className="skill-editor-head-actions">
                  <span className="agent-editor-state">{draft.enabled ? "Enabled" : "Disabled"}</span>
                  <div className="skill-editor-actions">
                    {sel !== "new" && (
                      <button className="btn-ghost danger agent-delete-action" type="button" onClick={remove}>
                        <Trash size={14} />
                        <span>{confirmDeleteId === selectedSkill?.id ? "Confirm delete" : "Delete"}</span>
                      </button>
                    )}
                    <button className="btn-accent" type="button" disabled={!canSave} onClick={save}>
                      {busy ? "Saving..." : "Save"}
                    </button>
                  </div>
                  {confirmDeleteId === selectedSkill?.id && <span className="agent-delete-note">Click again to delete this skill.</span>}
                </div>
              </div>

              <section className="agent-editor-section">
                <div className="agent-section-head">
                  <h4>State</h4>
                  <span>{draft.enabled ? "On" : "Off"}</span>
                </div>
                <Toggle checked={draft.enabled} onChange={(enabled) => setDraft((d) => ({ ...d, enabled }))} label="Enable automatic selection" />
              </section>

              {sel === "new" && (
                <section className="agent-editor-section">
                  <div className="agent-section-head">
                    <h4>Import</h4>
                    <span>{draft.mode}</span>
                  </div>
                  <div className="tool-mode-tabs" role="group" aria-label="Skill import mode">
                    {([
                      ["manual", "Manual"],
                      ["paste", "Paste"],
                      ["url", "GitHub URL"],
                    ] as const).map(([mode, label]) => (
                      <button
                        key={mode}
                        type="button"
                        className={"tool-mode-button" + (draft.mode === mode ? " active" : "")}
                        onClick={() => setDraft((d) => ({ ...d, mode }))}
                      >
                        {label}
                      </button>
                    ))}
                  </div>
                  {draft.mode === "paste" && (
                    <textarea
                      className="instr-input agent-instructions"
                      value={draft.skillMd}
                      onChange={(e) => setDraft((d) => ({ ...d, skillMd: e.target.value }))}
                      placeholder={"---\nname: my-skill\ndescription: When to use it\n---\nInstructions..."}
                    />
                  )}
                  {draft.mode === "url" && (
                    <label className="field agent-field">
                      <span>GitHub SKILL.md URL</span>
                      <input
                        className="css-input"
                        value={draft.skillUrl}
                        onChange={(e) => setDraft((d) => ({ ...d, skillUrl: e.target.value }))}
                        placeholder="https://github.com/owner/repo/tree/main/skill"
                      />
                    </label>
                  )}
                </section>
              )}

              {(sel !== "new" || draft.mode === "manual") && (
                <>
                  <section className="agent-editor-section">
                    <div className="agent-section-head">
                      <h4>Identity</h4>
                      <span>{draft.name.trim().length || 0} chars</span>
                    </div>
                    <label className="field agent-field">
                      <span>Name</span>
                      <input
                        className="css-input"
                        value={draft.name}
                        onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))}
                        placeholder="code-review"
                      />
                    </label>
                    <label className="field agent-field">
                      <span>Description</span>
                      <textarea
                        className="instr-input"
                        value={draft.description}
                        onChange={(e) => setDraft((d) => ({ ...d, description: e.target.value }))}
                        placeholder="Use when reviewing pull requests or diffs."
                      />
                    </label>
                  </section>

                  <section className="agent-editor-section skill-instructions-section">
                    <div className="agent-section-head">
                      <h4>Instructions</h4>
                      <span>{draft.instructions.trim().length} chars</span>
                    </div>
                    <textarea
                      className="instr-input agent-instructions"
                      value={draft.instructions}
                      onChange={(e) => setDraft((d) => ({ ...d, instructions: e.target.value }))}
                      placeholder="Review for correctness, regressions, missing tests..."
                    />
                  </section>
                </>
              )}

              {note && <span className="sheet-hint">{note}</span>}
            </div>
          ) : (
            <div className="agent-empty-state">
              <div className="agent-empty-icon" aria-hidden="true">
                <Lightbulb size={17} />
              </div>
              <h3>{skills.length ? "Select a skill" : "No skills yet"}</h3>
              <p>{skills.length ? "Choose a skill from the list." : "Create one manually, paste SKILL.md, or import from GitHub."}</p>
              <button className="btn-accent agent-header-action" type="button" onClick={() => edit("new")}>
                <Plus size={14} />
                <span>New skill</span>
              </button>
            </div>
          )}
        </section>
      </div>
    </SheetDialog>
  );
}
