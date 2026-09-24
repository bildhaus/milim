import { useEffect, useState } from "react";
import { deleteMcpServer, listMcpServers, MCP_PRESETS, saveMcpServer, testMcpServer, type McpEnvVar, type McpServerInfo } from "../api";
import { Cube, Plus, Trash, X } from "./icons";
import { SheetDialog } from "./SheetDialog";
import { PaneResizeHandle } from "./PaneResizeHandle";
import { MANAGER_DETAIL_MIN_WIDTH } from "../lib/paneSizes";
import { useSplitPane } from "../ui/usePaneResize";
import { Select, Toggle } from "./ui";
import "./McpManager.css";

type Selection = McpServerInfo | "new" | null;
type McpStatusTone = "ready" | "warning" | "error" | "off" | "draft";
type EnvDraft = McpEnvVar & { id: string };

function capabilitySummary(server: McpServerInfo): string {
  const caps = server.capabilities;
  if (!caps) return "tools";
  const names = [
    caps.tools ? "tools" : null,
    caps.resources ? "resources" : null,
    caps.prompts ? "prompts" : null,
    caps.apps ? "Apps" : null,
  ].filter(Boolean);
  return names.length ? names.join(", ") : "no advertised capabilities";
}

function serverStatus(server: McpServerInfo): { tone: McpStatusTone; label: string; detail: string } {
  if (server.missing_env?.length) return { tone: "warning", label: "Missing env", detail: `Missing required env: ${server.missing_env.join(", ")}` };
  if (!server.enabled) return { tone: "off", label: "Disabled", detail: "Saved but not exposed to agent runs." };
  if (server.error) return { tone: "error", label: "Error", detail: server.error };
  if (server.connected) {
    return {
      tone: "ready",
      label: "Connected",
      detail: `${server.tool_count} tool${server.tool_count === 1 ? "" : "s"} available to agents from ${capabilitySummary(server)}.`,
    };
  }
  return { tone: "warning", label: "Not connected", detail: "Saved, but no live stdio connection is active." };
}

function envDrafts(env?: McpEnvVar[]): EnvDraft[] {
  return (env ?? []).map((item, index) => ({
    id: `${item.key || "env"}-${index}`,
    key: item.key,
    value: item.secret ? "" : (item.value ?? ""),
    secret: Boolean(item.secret),
    required: Boolean(item.required),
    has_value: Boolean(item.has_value),
  }));
}

function apiEnv(env: EnvDraft[]): McpEnvVar[] {
  return env
    .map((item) => ({
      key: item.key.trim(),
      value: item.secret ? (item.value?.trim() ? item.value : undefined) : (item.value ?? ""),
      secret: Boolean(item.secret),
      required: Boolean(item.required),
    }))
    .filter((item) => item.key);
}

function argsSummary(args: string[]): string {
  if (args.length === 0) return "No arguments";
  if (args.length === 1) return args[0];
  return `${args.length} args`;
}

function noteTone(note: string): McpStatusTone {
  if (note.startsWith("Error:")) return "error";
  if (note.startsWith("Click Delete again")) return "warning";
  if (note.includes("Connecting") || note.includes("not connected")) return "warning";
  return "ready";
}

function McpListPlaceholder() {
  return (
    <div className="mcp-list-placeholder">
      <span>No MCP servers</span>
    </div>
  );
}

export function McpManager({ onClose }: { onClose: () => void }) {
  const rail = useSplitPane("mcpRail", "--manager-rail-width", MANAGER_DETAIL_MIN_WIDTH);
  const [servers, setServers] = useState<McpServerInfo[]>([]);
  const [sel, setSel] = useState<Selection>(null);
  const [name, setName] = useState("");
  const [command, setCommand] = useState("");
  const [argsText, setArgsText] = useState("");
  const [cwd, setCwd] = useState("");
  const [env, setEnv] = useState<EnvDraft[]>([]);
  const [enabled, setEnabled] = useState(true);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  const refresh = () => listMcpServers().then(setServers);
  useEffect(() => {
    refresh();
  }, []);

  function edit(s: McpServerInfo | "new") {
    setSel(s);
    setNote(null);
    setConfirmDeleteId(null);
    if (s === "new") {
      setName("");
      setCommand("");
      setArgsText("");
      setCwd("");
      setEnv([]);
      setEnabled(true);
    } else {
      setName(s.name);
      setCommand(s.command);
      setArgsText(s.args.join("\n"));
      setCwd(s.cwd ?? "");
      setEnv(envDrafts(s.env));
      setEnabled(s.enabled);
    }
  }

  function applyPreset(presetName: string) {
    const p = MCP_PRESETS.find((x) => x.name === presetName);
    if (!p) return;
    setConfirmDeleteId(null);
    setName(p.name);
    setCommand(p.command);
    setArgsText(p.args.join("\n"));
    if (p.note) setNote(p.note);
  }

  async function save() {
    if (!name.trim() || !command.trim()) return;
    setBusy(true);
    setConfirmDeleteId(null);
    setNote(enabled ? "Connecting... (first run may fetch the server package)" : "Saving disabled server...");
    const id = sel && sel !== "new" ? sel.id : undefined;
    const args = argsText
      .split("\n")
      .map((a) => a.trim())
      .filter(Boolean);
    const saved = await saveMcpServer({ id, name: name.trim(), command: command.trim(), args, cwd: cwd.trim() || null, env: apiEnv(env), enabled });
    setBusy(false);
    if (!saved) {
      setNote("Error: Failed to save MCP server.");
      return;
    }
    await refresh();
    setSel(saved);
    setNote(
      saved.error
        ? `Error: ${saved.error}`
        : saved.connected
          ? `Connected - ${saved.tool_count} tool${saved.tool_count === 1 ? "" : "s"} available`
          : enabled
            ? "Saved, but not connected."
            : "Saved (disabled).",
    );
  }

  async function testConnection() {
    if (!name.trim() || !command.trim()) return;
    setBusy(true);
    setConfirmDeleteId(null);
    setNote("Testing connection...");
    const id = sel && sel !== "new" ? sel.id : undefined;
    const args = argsText
      .split("\n")
      .map((a) => a.trim())
      .filter(Boolean);
    const result = await testMcpServer({ id, name: name.trim(), command: command.trim(), args, cwd: cwd.trim() || null, env: apiEnv(env), enabled });
    setBusy(false);
    if (!result) {
      setNote("Error: Failed to test MCP server.");
      return;
    }
    setNote(result.ok
      ? `Connection OK - ${result.tool_count} tool${result.tool_count === 1 ? "" : "s"} advertised`
      : `Error: ${result.error || (result.missing_env?.length ? `Missing env: ${result.missing_env.join(", ")}` : "Connection failed")}`);
  }

  async function remove() {
    if (!sel || sel === "new") return;
    if (confirmDeleteId !== sel.id) {
      setConfirmDeleteId(sel.id);
      setNote(`Click Delete again to remove "${sel.name}".`);
      return;
    }
    await deleteMcpServer(sel.id);
    await refresh();
    setConfirmDeleteId(null);
    setNote(null);
    setSel(null);
  }

  const connectedCount = servers.filter((s) => s.connected).length;
  const selectedServer = sel && sel !== "new" ? sel : null;
  const selectedStatus = selectedServer ? serverStatus(selectedServer) : null;
  const selectedPreset = MCP_PRESETS.find((p) => p.name === name && p.command === command && p.args.join("\n") === argsText.trim());
  const args = argsText
    .split("\n")
    .map((a) => a.trim())
    .filter(Boolean);
  const canSave = Boolean(name.trim() && command.trim() && !busy);
  const canTest = Boolean(name.trim() && command.trim() && !busy);
  const editorTitle = sel === "new" ? "New MCP server" : name.trim() || selectedServer?.name || "Select an MCP server";
  const updateEnv = (id: string, patch: Partial<EnvDraft>) =>
    setEnv((rows) => rows.map((row) => row.id === id ? { ...row, ...patch } : row));
  const addEnv = () =>
    setEnv((rows) => [...rows, { id: `env-${Date.now()}`, key: "", value: "", secret: false, required: false, has_value: false }]);

  return (
    <SheetDialog title="MCP Servers" className="sheet agents-sheet mcp-manager-sheet" resizable={{ id: "mcp" }} onClose={onClose}>
        <div className="mcp-manager-header">
          <div className="mcp-manager-title">
            <h2>MCP Servers</h2>
            <p>
              Connect external Model Context Protocol servers (stdio). Their tools become available to the agent
              automatically. On Windows, <code>npx</code>/<code>uvx</code> resolve via the shell.
            </p>
          </div>
          <div className="mcp-manager-header-actions">
            <button className="btn-accent mcp-header-action" type="button" onClick={() => edit("new")}>
              <Plus size={14} />
              <span>Add server</span>
            </button>
            <button className="icon-btn sheet-close mcp-close" type="button" onClick={onClose} title="Close" aria-label="Close MCP servers">
              <X size={15} />
            </button>
          </div>
        </div>

        <div ref={rail.containerRef} className="mcp-manager-body" style={rail.style}>
          <PaneResizeHandle resize={rail.resize} className="manager-rail-resize-handle" data-testid="mcp-rail-resize-handle" />
          <aside className="mcp-rail" aria-label="MCP server list">
            <div className="mcp-rail-summary">
              <span>{servers.length} saved</span>
              <span>{connectedCount} connected</span>
            </div>
            <button className="mcp-rail-action" type="button" onClick={() => edit("new")}>
              <Plus size={14} />
              <span>New</span>
            </button>
            {servers.length > 0 ? (
              <div className="mcp-list" role="list">
                {servers.map((s) => {
                  const status = serverStatus(s);
                  return (
                    <button
                      key={s.id}
                      type="button"
                      className={"mcp-list-row" + (selectedServer?.id === s.id ? " active" : "")}
                      onClick={() => edit(s)}
                    >
                      <span className={"mcp-status-dot " + status.tone} aria-hidden="true" />
                      <span className="mcp-row-copy">
                        <span className="mcp-row-name">{s.name}</span>
                        <span className="mcp-row-command">{s.command}</span>
                        <span className="mcp-row-foot">
                          <span>{status.label}</span>
                          <span>{s.connected ? `${s.tool_count} tools` : argsSummary(s.args)}</span>
                        </span>
                      </span>
                    </button>
                  );
                })}
              </div>
            ) : (
              <McpListPlaceholder />
            )}
          </aside>

          <main className="mcp-detail">
            {sel ? (
              <div className="mcp-editor">
                <div className="mcp-editor-head">
                  <div>
                    <span className="mcp-editor-kicker">{sel === "new" ? "Draft server" : "Server connection"}</span>
                    <h3>{editorTitle}</h3>
                  </div>
                  <span className={"mcp-editor-state " + (selectedStatus?.tone ?? "draft")}>{selectedStatus?.label ?? "Draft"}</span>
                </div>

                <div className="mcp-impact-panel" aria-label="MCP run impact">
                  <div className="mcp-impact-item">
                    <span>Connection</span>
                    <strong>{selectedStatus?.label ?? "Draft"}</strong>
                    <em>{selectedStatus?.detail ?? "Choose a preset or enter a stdio command."}</em>
                  </div>
                  <div className="mcp-impact-item">
                    <span>Command</span>
                    <strong>{command.trim() || "Required"}</strong>
                    <em>{selectedPreset ? `${selectedPreset.name} preset` : "Manual stdio command"}</em>
                  </div>
                  <div className="mcp-impact-item">
                    <span>Arguments</span>
                    <strong>{argsSummary(args)}</strong>
                    <em>{args.length ? "Sent one per line" : "No process arguments"}</em>
                  </div>
                  <div className="mcp-impact-item">
                    <span>Environment</span>
                    <strong>{env.length ? `${env.length} var${env.length === 1 ? "" : "s"}` : "None"}</strong>
                    <em>{cwd.trim() ? `cwd: ${cwd.trim()}` : "Default working directory"}</em>
                  </div>
                  <div className="mcp-impact-item">
                    <span>Tools</span>
                    <strong>{selectedServer ? `${selectedServer.tool_count} tool${selectedServer.tool_count === 1 ? "" : "s"}` : "After connect"}</strong>
                    <em>{selectedServer ? capabilitySummary(selectedServer) : "Available after save"}</em>
                  </div>
                </div>

                <section className="mcp-editor-section">
                  <div className="mcp-section-head">
                    <h4>Preset</h4>
                    <span>{selectedPreset?.name ?? "Optional"}</span>
                  </div>
                  <Select
                    value={selectedPreset?.name ?? ""}
                    placeholder="Choose a preset..."
                    options={MCP_PRESETS.map((p) => ({ label: p.name, value: p.name }))}
                    onChange={applyPreset}
                  />
                </section>

                <section className="mcp-editor-section">
                  <div className="mcp-section-head">
                    <h4>Identity</h4>
                    <span>{name.trim() || "Unnamed"}</span>
                  </div>
                  <label className="field mcp-field">
                    <span>Name</span>
                    <input className="css-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Filesystem" />
                  </label>
                </section>

                <section className="mcp-editor-section">
                  <div className="mcp-section-head">
                    <h4>Command</h4>
                    <span>{command.trim() || "Required"}</span>
                  </div>
                  <label className="field mcp-field">
                    <span>Command</span>
                    <input
                      className="css-input mcp-command-input"
                      value={command}
                      onChange={(e) => setCommand(e.target.value)}
                      placeholder="npx"
                    />
                  </label>
                  <label className="field mcp-field">
                    <span>Arguments (one per line)</span>
                    <textarea
                      className="instr-input mcp-args-input"
                      value={argsText}
                      onChange={(e) => setArgsText(e.target.value)}
                      placeholder={"-y\n@modelcontextprotocol/server-filesystem\nC:\\Users\\me\\project"}
                    />
                  </label>
                  <label className="field mcp-field">
                    <span>Working directory</span>
                    <input
                      className="css-input"
                      value={cwd}
                      onChange={(e) => setCwd(e.target.value)}
                      placeholder="Optional cwd for the MCP process"
                    />
                  </label>
                </section>

                <section className="mcp-editor-section">
                  <div className="mcp-section-head">
                    <h4>Environment</h4>
                    <button className="section-icon-btn" type="button" title="Add env var" onClick={addEnv}>
                      <Plus size={12} />
                    </button>
                  </div>
                  <div className="mcp-env-list">
                    {env.length === 0 ? (
                      <span className="mcp-env-empty">No env vars</span>
                    ) : env.map((item) => (
                      <div className="mcp-env-row" key={item.id}>
                        <input
                          className="css-input"
                          value={item.key}
                          onChange={(e) => updateEnv(item.id, { key: e.target.value })}
                          placeholder="ENV_KEY"
                        />
                        <input
                          className="css-input"
                          type={item.secret ? "password" : "text"}
                          value={item.value ?? ""}
                          onChange={(e) => updateEnv(item.id, { value: e.target.value })}
                          placeholder={item.secret && item.has_value ? "Saved secret - enter to replace" : "Value"}
                        />
                        <Toggle checked={Boolean(item.secret)} onChange={(checked) => updateEnv(item.id, { secret: checked, required: checked ? true : item.required })} label="Secret" />
                        <Toggle checked={Boolean(item.required)} onChange={(checked) => updateEnv(item.id, { required: checked })} label="Required" />
                        <button className="icon-btn" type="button" title="Remove env var" onClick={() => setEnv((rows) => rows.filter((row) => row.id !== item.id))}>
                          <Trash size={13} />
                        </button>
                      </div>
                    ))}
                  </div>
                </section>

                <section className="mcp-editor-section">
                  <div className="mcp-section-head">
                    <h4>Status</h4>
                    <span>{enabled ? "Enabled" : "Disabled"}</span>
                  </div>
                  <div className="mcp-status-grid">
                    <Toggle checked={enabled} onChange={setEnabled} label="Enabled" />
                    <span>{enabled ? "Connect and expose tools when available." : "Keep this server saved but inactive."}</span>
                  </div>
                  {note && <p className={"mcp-note " + noteTone(note)}>{note}</p>}
                </section>

                <div className="mcp-action-footer">
                  {sel !== "new" && (
                    <button className="btn-ghost danger mcp-delete-action" type="button" disabled={busy} onClick={remove}>
                      <Trash size={14} />
                      <span>{confirmDeleteId === selectedServer?.id ? "Confirm delete" : "Delete"}</span>
                    </button>
                  )}
                  <span className="spacer" />
                  <button className="btn-ghost" type="button" disabled={!canTest} onClick={testConnection}>
                    Test connection
                  </button>
                  <button className="btn-accent" type="button" disabled={!canSave} onClick={save}>
                    {busy ? "Connecting..." : "Save & connect"}
                  </button>
                </div>
              </div>
            ) : (
              <div className="mcp-empty-state">
                <div className="mcp-empty-icon" aria-hidden="true">
                  <Cube size={18} />
                </div>
                <h3>{servers.length ? "Select an MCP server" : "No MCP servers yet"}</h3>
                <p>
                  {servers.length
                    ? "Choose a saved server from the list, or connect another stdio tool source."
                    : "Add a preset or custom stdio command to expose external tools to agents."}
                </p>
                <button className="btn-accent mcp-header-action" type="button" onClick={() => edit("new")}>
                  <Plus size={14} />
                  <span>Add server</span>
                </button>
              </div>
            )}
          </main>
        </div>
      </SheetDialog>
  );
}
