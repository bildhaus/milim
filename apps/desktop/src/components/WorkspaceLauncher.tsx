import type { ReactNode } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  listWorkspaceLaunchers,
  openWorkspaceLauncher,
  type WorkspaceLauncher,
  type WorkspaceLauncherId,
} from "../api";
import { rankedWorkspaceLaunchers } from "../lib/workspaceLauncher";
import { useUiPreferences } from "../ui/store";
import { Code, ExternalLink, FolderOpen, GitBranch, Terminal } from "./icons";

function launcherIcon(id: WorkspaceLauncherId): ReactNode {
  switch (id) {
    case "file_manager":
      return <FolderOpen size={14} />;
    case "terminal":
    case "wsl":
      return <Terminal size={14} />;
    case "git_bash":
      return <GitBranch size={14} />;
    case "vscode":
    case "zed":
    case "android_studio":
      return <Code size={14} />;
  }
}

export function WorkspaceLauncherButton({
  folder,
  variant = "panel",
}: {
  folder: string;
  variant?: "panel" | "quick-summary";
}) {
  const [open, setOpen] = useState(false);
  const [launchers, setLaunchers] = useState<WorkspaceLauncher[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [launchingId, setLaunchingId] = useState<WorkspaceLauncherId | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const lastUsedByFolder = useUiPreferences((s) => s.workspaceLauncherLastUsedByFolder);
  const rememberWorkspaceLauncher = useUiPreferences((s) => s.rememberWorkspaceLauncher);
  const launcherPreference = useUiPreferences((s) => s.workspaceLauncherPreference);
  const pushNotice = useUiPreferences((s) => s.pushNotice);
  const unavailableNoticeRef = useRef("");
  const activeFolder = folder.trim();

  useEffect(() => {
    if (!open) return;
    const onDoc = (event: globalThis.MouseEvent) => {
      const target = event.target;
      if (ref.current && target instanceof Node && !ref.current.contains(target)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  useEffect(() => {
    if (!open || !activeFolder) {
      setLaunchers([]);
      setError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    void listWorkspaceLaunchers(activeFolder)
      .then((items) => {
        if (!cancelled) setLaunchers(items);
      })
      .catch((e) => {
        if (!cancelled) {
          setLaunchers([]);
          setError(e instanceof Error ? e.message : String(e));
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [activeFolder, open]);

  const rankedLaunchers = useMemo(() => {
    const ranked = rankedWorkspaceLaunchers(launchers, activeFolder, lastUsedByFolder);
    if (launcherPreference === "remember") return ranked;
    const preferred = ranked.find((launcher) => launcher.id === launcherPreference);
    return preferred?.available ? [preferred, ...ranked.filter((launcher) => launcher.id !== preferred.id)] : ranked;
  }, [activeFolder, lastUsedByFolder, launcherPreference, launchers]);
  useEffect(() => {
    if (!open || loading || launcherPreference === "remember") return;
    const preferred = launchers.find((launcher) => launcher.id === launcherPreference);
    if (!preferred || preferred.available) return;
    const key = `${activeFolder}:${launcherPreference}`;
    if (unavailableNoticeRef.current === key) return;
    unavailableNoticeRef.current = key;
    pushNotice({ tone: "warning", message: `${preferred.label} is unavailable. Showing Milim's recommended opener instead.` });
  }, [activeFolder, launcherPreference, launchers, loading, open, pushNotice]);
  const visibleLaunchers = useMemo(() => {
    const available = rankedLaunchers.filter((launcher) => launcher.available);
    return available.length ? available : rankedLaunchers;
  }, [rankedLaunchers]);

  async function launchWorkspace(launcher: WorkspaceLauncher) {
    if (!activeFolder || !launcher.available || launchingId) return;
    setLaunchingId(launcher.id);
    setError(null);
    try {
      await openWorkspaceLauncher(activeFolder, launcher.id);
      if (launcherPreference === "remember") rememberWorkspaceLauncher(activeFolder, launcher.id);
      setOpen(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLaunchingId(null);
    }
  }

  const buttonClass = [
    variant === "quick-summary" ? "icon-btn workspace-launcher-quick-btn" : "workspace-launcher-btn",
    open ? "active" : null,
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={`workspace-launcher-wrap ${variant}`} ref={ref}>
      <button
        type="button"
        className={buttonClass}
        data-testid="workspace-launcher-trigger"
        title={activeFolder ? `Open workspace: ${activeFolder}` : "Choose a folder first"}
        aria-label="Open workspace in another app"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={(event) => {
          event.stopPropagation();
          setOpen((value) => !value);
        }}
      >
        <ExternalLink size={14} />
      </button>
      <div
        className={"menu workspace-launcher-menu" + (open ? " open" : "")}
        role="menu"
        aria-label="Open workspace"
        aria-hidden={!open}
      >
        {!activeFolder ? (
          <div className="workspace-launcher-state">Choose a folder first.</div>
        ) : loading ? (
          <div className="workspace-launcher-state">Finding apps...</div>
        ) : (
          <>
            {visibleLaunchers.map((launcher, index) => (
              <button
                key={launcher.id}
                type="button"
                role="menuitem"
                className={"menu-item workspace-launcher-item" + (index === 0 && launcher.recommendedReason ? " recommended" : "")}
                disabled={!launcher.available || launchingId === launcher.id}
                title={launcher.reason ?? launcher.recommendedReason ?? launcher.label}
                onClick={(event) => {
                  event.stopPropagation();
                  void launchWorkspace(launcher);
                }}
              >
                <span className="workspace-launcher-icon">{launcherIcon(launcher.id)}</span>
                <span className="workspace-launcher-label">{launcher.label}</span>
                {index === 0 && launcher.recommendedReason && (
                  <span className="workspace-launcher-meta">Suggested</span>
                )}
              </button>
            ))}
            {!visibleLaunchers.length && (
              <div className="workspace-launcher-state">No launchers found.</div>
            )}
            {error && <div className="workspace-launcher-error">{error}</div>}
          </>
        )}
      </div>
    </div>
  );
}
