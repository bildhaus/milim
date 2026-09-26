import { useEffect, useState } from "react";
import {
  deleteMcpServer,
  listMcpServers,
  MCP_PRESETS,
  openExternalUrl,
  reconnectMcpServer,
  saveMcpServer,
  signOutMcpServer,
  startMcpServerSignIn,
  testMcpServer,
  type McpEnvVar,
  type McpServerDraft,
  type McpServerInfo,
  type McpTransportKind,
} from "../api";
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

const DEFAULT_CALL_TIMEOUT_SECS = 60;
const MAX_CALL_TIMEOUT_SECS = 600;
const TRANSPORT_OPTIONS = [
  { label: "Local command (stdio)", value: "stdio" },
  { label: "Remote URL (Streamable HTTP)", value: "http" },
];

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
  if (server.missing_env?.length) return { tone: "warning", label: "Missing values", detail: `Missing required values: ${server.missing_env.join(", ")}` };
  if (!server.enabled) return { tone: "off", label: "Disabled", detail: "Saved but not exposed to agent runs." };
  switch (server.status) {
    case "auth_required":
      return { tone: "warning", label: "Sign-in required", detail: "Sign in to let milim call this server's tools." };
    case "connecting":
      return { tone: "warning", label: "Connecting", detail: "Starting the connection..." };
    case "reconnecting": {
      const retry = server.retry_in_secs != null ? ` Next attempt in ${server.retry_in_secs}s.` : "";
      return { tone: "warning", label: "Reconnecting", detail: `Connection lost${server.error ? `: ${server.error}` : ""}. Its tools are withheld until it reconnects.${retry}` };
    }
    default:
      break;
  }
  if (server.error) return { tone: "error", label: "Error", detail: server.error };
  if (server.connected) {
    return {
      tone: "ready",
      label: "Connected",
      detail: `${server.tool_count} tool${server.tool_count === 1 ? "" : "s"} available to agents from ${capabilitySummary(server)}.`,
    };
  }
  return { tone: "warning", label: "Not connected", detail: "Saved, but no live connection is active." };
}

function isSettling(server: McpServerInfo): boolean {
  return server.status === "connecting" || server.status === "reconnecting" || server.auth?.flow?.status === "pending";
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

function serverTarget(server: McpServerInfo): string {
  return server.type === "http" ? (server.url ?? "") : server.command;
}

function noteTone(note: string): McpStatusTone {
  if (note.startsWith("Error:")) return "error";
  if (note.startsWith("Click Delete again")) return "warning";
  if (note.includes("Connecting") || note.includes("not connected") || note.includes("Sign in") || note.includes("browser")) return "warning";
  return "ready";
}

function McpListPlaceholder() {
  return (
    <div className="mcp-list-placeholder">
      <span>No MCP servers</span>
    </div>
  );
}

function KeyValueRows({
  rows,
  onChange,
  emptyLabel,
  keyPlaceholder,
  removeLabel,
}: {
  rows: EnvDraft[];
  onChange: (rows: EnvDraft[]) => void;
  emptyLabel: string;
  keyPlaceholder: string;
  removeLabel: string;
}) {
  const update = (id: string, patch: Partial<EnvDraft>) => onChange(rows.map((row) => (row.id === id ? { ...row, ...patch } : row)));
  return (
    <div className="mcp-env-list">
      {rows.length === 0 ? (
        <span className="mcp-env-empty">{emptyLabel}</span>
      ) : rows.map((item) => (
        <div className="mcp-env-row" key={item.id}>
          <input className="css-input" value={item.key} onChange={(e) => update(item.id, { key: e.target.value })} placeholder={keyPlaceholder} />
          <input
            className="css-input"
            type={item.secret ? "password" : "text"}
            value={item.value ?? ""}
            onChange={(e) => update(item.id, { value: e.target.value })}
            placeholder={item.secret && item.has_value ? "Saved secret - enter to replace" : "Value"}
          />
          <Toggle checked={Boolean(item.secret)} onChange={(checked) => update(item.id, { secret: checked, required: checked ? true : item.required })} label="Secret" />
          <Toggle checked={Boolean(item.required)} onChange={(checked) => update(item.id, { required: checked })} label="Required" />
          <button className="icon-btn" type="button" title={removeLabel} onClick={() => onChange(rows.filter((row) => row.id !== item.id))}>
            <Trash size={13} />
          </button>
        </div>
      ))}
    </div>
  );
}

export function McpManager({ onClose }: { onClose: () => void }) {
  const rail = useSplitPane("mcpRail", "--manager-rail-width", MANAGER_DETAIL_MIN_WIDTH);
  const [servers, setServers] = useState<McpServerInfo[]>([]);
  const [sel, setSel] = useState<Selection>(null);
  const [name, setName] = useState("");
  const [transport, setTransport] = useState<McpTransportKind>("stdio");
  const [command, setCommand] = useState("");
  const [argsText, setArgsText] = useState("");
  const [cwd, setCwd] = useState("");
  const [env, setEnv] = useState<EnvDraft[]>([]);
  const [url, setUrl] = useState("");
  const [headers, setHeaders] = useState<EnvDraft[]>([]);
  const [oauthClientId, setOauthClientId] = useState("");
  const [trustHints, setTrustHints] = useState(false);
  const [timeoutText, setTimeoutText] = useState(String(DEFAULT_CALL_TIMEOUT_SECS));
  const [enabled, setEnabled] = useState(true);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  const refresh = () => listMcpServers().then(setServers);
  useEffect(() => {
    refresh();
  }, []);

  const settling = servers.some(isSettling);
  useEffect(() => {
    if (!settling) return;
    const timer = window.setInterval(refresh, 2000);
    return () => window.clearInterval(timer);
  }, [settling]);

  function edit(s: McpServerInfo | "new") {
    setSel(s);
    setNote(null);
    setConfirmDeleteId(null);
    if (s === "new") {
      setName("");
      setTransport("stdio");
      setCommand("");
      setArgsText("");
      setCwd("");
      setEnv([]);
      setUrl("");
      setHeaders([]);
      setOauthClientId("");
      setTrustHints(false);
      setTimeoutText(String(DEFAULT_CALL_TIMEOUT_SECS));
      setEnabled(true);
    } else {
      setName(s.name);
      setTransport(s.type ?? "stdio");
      setCommand(s.command);
      setArgsText(s.args.join("\n"));
      setCwd(s.cwd ?? "");
      setEnv(envDrafts(s.env));
      setUrl(s.url ?? "");
      setHeaders(envDrafts(s.headers));
      setOauthClientId(s.oauth_client_id ?? "");
      setTrustHints(Boolean(s.trust_read_only_hints));
      setTimeoutText(String(s.call_timeout_secs ?? DEFAULT_CALL_TIMEOUT_SECS));
      setEnabled(s.enabled);
    }
  }

  function applyPreset(presetName: string) {
    const p = MCP_PRESETS.find((x) => x.name === presetName);
    if (!p) return;
    setConfirmDeleteId(null);
    setName(p.name);
    setTransport("stdio");
    setCommand(p.command);
    setArgsText(p.args.join("\n"));
    if (p.note) setNote(p.note);
  }

  const args = argsText
    .split("\n")
    .map((a) => a.trim())
    .filter(Boolean);
  const timeoutSecs = Number.parseInt(timeoutText, 10);
  const timeoutValid = Number.isFinite(timeoutSecs) && timeoutSecs >= 1 && timeoutSecs <= MAX_CALL_TIMEOUT_SECS;
  const target = transport === "http" ? url.trim() : command.trim();

  function draft(): McpServerDraft {
    return {
      id: sel && sel !== "new" ? sel.id : undefined,
      name: name.trim(),
      type: transport,
      command: transport === "stdio" ? command.trim() : "",
      args: transport === "stdio" ? args : [],
      cwd: transport === "stdio" ? cwd.trim() || null : null,
      env: transport === "stdio" ? apiEnv(env) : [],
      url: transport === "http" ? url.trim() : null,
      headers: transport === "http" ? apiEnv(headers) : [],
      enabled,
      trust_read_only_hints: trustHints,
      call_timeout_secs: timeoutValid ? timeoutSecs : null,
      oauth_client_id: transport === "http" ? oauthClientId.trim() || null : null,
    };
  }

  function connectionNote(server: McpServerInfo): string {
    if (server.status === "auth_required") return "Saved. Sign in to connect this server.";
    if (server.error) return `Error: ${server.error}`;
    if (server.connected) return `Connected - ${server.tool_count} tool${server.tool_count === 1 ? "" : "s"} available`;
    return server.enabled ? "Saved, but not connected." : "Saved (disabled).";
  }

  async function save() {
    if (!name.trim() || !target) return;
    setBusy(true);
    setConfirmDeleteId(null);
    setNote(enabled ? (transport === "stdio" ? "Connecting... (first run may fetch the server package)" : "Connecting...") : "Saving disabled server...");
    const saved = await saveMcpServer(draft());
    setBusy(false);
    if (!saved) {
      setNote("Error: Failed to save MCP server.");
      return;
    }
    await refresh();
    setSel(saved);
    setNote(connectionNote(saved));
  }

  async function testConnection() {
    if (!name.trim() || !target) return;
    setBusy(true);
    setConfirmDeleteId(null);
    setNote("Testing connection...");
    const result = await testMcpServer(draft());
    setBusy(false);
    if (!result) {
      setNote("Error: Failed to test MCP server.");
      return;
    }
    if (result.auth_required) {
      setNote("Server reachable. Save it, then Sign in to finish connecting.");
      return;
    }
    setNote(result.ok
      ? `Connection OK - ${result.tool_count} tool${result.tool_count === 1 ? "" : "s"} advertised`
      : `Error: ${result.error || (result.missing_env?.length ? `Missing values: ${result.missing_env.join(", ")}` : "Connection failed")}`);
  }

  async function reconnect() {
    if (!selectedServer) return;
    setBusy(true);
    setNote("Reconnecting...");
    try {
      const server = await reconnectMcpServer(selectedServer.id);
      await refresh();
      if (server) setNote(connectionNote(server));
    } catch (error) {
      setNote(`Error: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setBusy(false);
    }
  }

  async function signIn() {
    if (!selectedServer) return;
    setBusy(true);
    setNote("Starting sign-in...");
    try {
      const flow = await startMcpServerSignIn(selectedServer.id);
      if (flow.url) await openExternalUrl(flow.url);
      setNote("Finish signing in in your browser. This server connects automatically afterwards.");
      await refresh();
    } catch (error) {
      setNote(`Error: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setBusy(false);
    }
  }

  async function signOut() {
    if (!selectedServer) return;
    setBusy(true);
    try {
      await signOutMcpServer(selectedServer.id);
      await refresh();
      setNote("Signed out.");
    } catch (error) {
      setNote(`Error: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setBusy(false);
    }
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
  const selectedServer = sel && sel !== "new" ? (servers.find((s) => s.id === sel.id) ?? sel) : null;
  const selectedStatus = selectedServer ? serverStatus(selectedServer) : null;
  const selectedPreset = transport === "stdio" ? MCP_PRESETS.find((p) => p.name === name && p.command === command && p.args.join("\n") === argsText.trim()) : undefined;
  const canSave = Boolean(name.trim() && target && timeoutValid && !busy);
  const canTest = canSave;
  const editorTitle = sel === "new" ? "New MCP server" : name.trim() || selectedServer?.name || "Select an MCP server";
  const savedHttp = selectedServer?.type === "http";
  const authStatus = selectedServer?.auth?.status ?? "not_required";
  const authFlow = selectedServer?.auth?.flow;
  const declaredReadOnly = selectedServer?.declared_read_only_tools ?? 0;
  const logs = selectedServer?.logs ?? [];
  const newRow = (prefix: string): EnvDraft => ({ id: `${prefix}-${Date.now()}`, key: "", value: "", secret: false, required: false, has_value: false });

  return (
    <SheetDialog title="MCP Servers" className="sheet agents-sheet mcp-manager-sheet" resizable={{ id: "mcp" }} onClose={onClose}>
        <div className="mcp-manager-header">
          <div className="mcp-manager-title">
            <h2>MCP Servers</h2>
            <p>
              Connect external Model Context Protocol servers by local command (stdio) or remote URL (Streamable HTTP, with
              OAuth sign-in). Their tools become available to the agent automatically. On Windows, <code>npx</code>/<code>uvx</code> resolve via the shell.
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
                        <span className="mcp-row-command">{serverTarget(s)}</span>
                        <span className="mcp-row-foot">
                          <span>{status.label}</span>
                          <span>{s.connected ? `${s.tool_count} tools` : s.type === "http" ? "HTTP" : argsSummary(s.args)}</span>
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
                    <em>{selectedStatus?.detail ?? (transport === "http" ? "Enter the server's MCP URL." : "Choose a preset or enter a stdio command.")}</em>
                  </div>
                  <div className="mcp-impact-item">
                    <span>{transport === "http" ? "URL" : "Command"}</span>
                    <strong>{target || "Required"}</strong>
                    <em>{transport === "http" ? "Streamable HTTP, legacy SSE fallback" : selectedPreset ? `${selectedPreset.name} preset` : "Manual stdio command"}</em>
                  </div>
                  {transport === "stdio" ? (
                    <div className="mcp-impact-item">
                      <span>Arguments</span>
                      <strong>{argsSummary(args)}</strong>
                      <em>{args.length ? "Sent one per line" : "No process arguments"}</em>
                    </div>
                  ) : (
                    <div className="mcp-impact-item">
                      <span>Sign-in</span>
                      <strong>{authStatus === "signed_in" ? "Signed in" : authStatus === "required" ? "Required" : "Not required"}</strong>
                      <em>OAuth with PKCE when the server asks for it</em>
                    </div>
                  )}
                  <div className="mcp-impact-item">
                    <span>{transport === "http" ? "Headers" : "Environment"}</span>
                    <strong>
                      {transport === "http"
                        ? (headers.length ? `${headers.length} header${headers.length === 1 ? "" : "s"}` : "None")
                        : (env.length ? `${env.length} var${env.length === 1 ? "" : "s"}` : "None")}
                    </strong>
                    <em>{transport === "http" ? "Secret values stay encrypted" : cwd.trim() ? `cwd: ${cwd.trim()}` : "Default working directory"}</em>
                  </div>
                  <div className="mcp-impact-item">
                    <span>Tools</span>
                    <strong>{selectedServer ? `${selectedServer.tool_count} tool${selectedServer.tool_count === 1 ? "" : "s"}` : "After connect"}</strong>
                    <em>{selectedServer ? capabilitySummary(selectedServer) : "Available after save"}</em>
                  </div>
                </div>

                <section className="mcp-editor-section">
                  <div className="mcp-section-head">
                    <h4>Transport</h4>
                    <span>{transport === "http" ? "Remote" : "Local"}</span>
                  </div>
                  <Select
                    value={transport}
                    ariaLabel="MCP transport"
                    options={TRANSPORT_OPTIONS}
                    onChange={(value) => setTransport(value === "http" ? "http" : "stdio")}
                  />
                </section>

                {transport === "stdio" && (
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
                )}

                <section className="mcp-editor-section">
                  <div className="mcp-section-head">
                    <h4>Identity</h4>
                    <span>{name.trim() || "Unnamed"}</span>
                  </div>
                  <label className="field mcp-field">
                    <span>Name</span>
                    <input className="css-input" value={name} onChange={(e) => setName(e.target.value)} placeholder={transport === "http" ? "Remote tools" : "Filesystem"} />
                  </label>
                </section>

                {transport === "stdio" ? (
                  <>
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
                        <button className="section-icon-btn" type="button" title="Add env var" onClick={() => setEnv((rows) => [...rows, newRow("env")])}>
                          <Plus size={12} />
                        </button>
                      </div>
                      <KeyValueRows rows={env} onChange={setEnv} emptyLabel="No env vars" keyPlaceholder="ENV_KEY" removeLabel="Remove env var" />
                    </section>
                  </>
                ) : (
                  <>
                    <section className="mcp-editor-section">
                      <div className="mcp-section-head">
                        <h4>Endpoint</h4>
                        <span>{url.trim() ? "Streamable HTTP" : "Required"}</span>
                      </div>
                      <label className="field mcp-field">
                        <span>Server URL</span>
                        <input
                          className="css-input mcp-command-input"
                          value={url}
                          onChange={(e) => setUrl(e.target.value)}
                          placeholder="https://mcp.example.com/mcp"
                        />
                      </label>
                      <label className="field mcp-field">
                        <span>OAuth client ID</span>
                        <input
                          className="css-input"
                          value={oauthClientId}
                          onChange={(e) => setOauthClientId(e.target.value)}
                          placeholder="Optional - only for servers without dynamic client registration"
                        />
                      </label>
                    </section>

                    <section className="mcp-editor-section">
                      <div className="mcp-section-head">
                        <h4>Headers</h4>
                        <button className="section-icon-btn" type="button" title="Add header" onClick={() => setHeaders((rows) => [...rows, newRow("header")])}>
                          <Plus size={12} />
                        </button>
                      </div>
                      <KeyValueRows rows={headers} onChange={setHeaders} emptyLabel="No headers" keyPlaceholder="Header-Name" removeLabel="Remove header" />
                    </section>

                    {savedHttp && (authStatus !== "not_required" || authFlow) && (
                      <section className="mcp-editor-section">
                        <div className="mcp-section-head">
                          <h4>Sign-in</h4>
                          <span>{authStatus === "signed_in" ? "Signed in" : authStatus === "required" ? "Required" : "Not required"}</span>
                        </div>
                        <div className="mcp-status-grid">
                          {authStatus === "signed_in" ? (
                            <button className="btn-ghost" type="button" disabled={busy} onClick={signOut}>Sign out</button>
                          ) : (
                            <button className="btn-accent" type="button" disabled={busy || !selectedServer?.enabled} onClick={signIn}>Sign in</button>
                          )}
                          <span>
                            {authFlow?.status === "pending"
                              ? "Waiting for the browser sign-in to finish..."
                              : authFlow?.status === "error"
                                ? `Last sign-in failed: ${authFlow.error ?? "unknown error"}`
                                : authStatus === "signed_in"
                                  ? "Tokens are stored encrypted and refresh automatically."
                                  : "Opens your browser to sign in with OAuth."}
                          </span>
                        </div>
                      </section>
                    )}
                  </>
                )}

                <section className="mcp-editor-section">
                  <div className="mcp-section-head">
                    <h4>Tool approval</h4>
                    <span>{trustHints ? "Trusted hints" : "Hints untrusted"}</span>
                  </div>
                  <div className="mcp-status-grid">
                    <Toggle checked={trustHints} onChange={setTrustHints} label="Trust this server's read-only hints" />
                    <span>
                      {trustHints
                        ? "Tools this server marks read-only skip approval and are offered in Guarded mode."
                        : "Tools still need approval even when this server marks them read-only, and Guarded mode withholds them."}
                      {selectedServer && declaredReadOnly > 0 ? ` ${declaredReadOnly} tool${declaredReadOnly === 1 ? "" : "s"} declared read-only.` : ""}
                    </span>
                  </div>
                  <label className="field mcp-field">
                    <span>Tool call timeout (seconds)</span>
                    <input
                      className="css-input"
                      type="number"
                      min={1}
                      max={MAX_CALL_TIMEOUT_SECS}
                      value={timeoutText}
                      onChange={(e) => setTimeoutText(e.target.value)}
                    />
                  </label>
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

                {logs.length > 0 && (
                  <section className="mcp-editor-section">
                    <div className="mcp-section-head">
                      <h4>Diagnostics</h4>
                      <span>{logs.length} recent</span>
                    </div>
                    <ol className="mcp-log-list">
                      {logs.slice().reverse().map((entry, index) => (
                        <li key={`${entry.at_ms}-${index}`} className={"mcp-log-entry " + entry.level}>
                          <time>{new Date(entry.at_ms).toLocaleTimeString()}</time>
                          <strong>{entry.level}</strong>
                          <span>{entry.logger ? `${entry.logger}: ${entry.message}` : entry.message}</span>
                        </li>
                      ))}
                    </ol>
                  </section>
                )}

                <div className="mcp-action-footer">
                  {sel !== "new" && (
                    <button className="btn-ghost danger mcp-delete-action" type="button" disabled={busy} onClick={remove}>
                      <Trash size={14} />
                      <span>{confirmDeleteId === selectedServer?.id ? "Confirm delete" : "Delete"}</span>
                    </button>
                  )}
                  <span className="spacer" />
                  {selectedServer?.enabled && (
                    <button className="btn-ghost" type="button" disabled={busy} onClick={reconnect}>
                      Reconnect
                    </button>
                  )}
                  <button className="btn-ghost" type="button" disabled={!canTest} onClick={testConnection}>
                    Test connection
                  </button>
                  <button className="btn-accent" type="button" disabled={!canSave} onClick={save}>
                    {busy ? "Working..." : "Save & connect"}
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
                    ? "Choose a saved server from the list, or connect another tool source."
                    : "Add a preset, a local stdio command, or a remote MCP URL to expose external tools to agents."}
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
