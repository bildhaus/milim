import { getWorkspaceGitStatus, runWorkspaceGitAction, setWorkspace } from "../api";
import { useSessions, type ThreadSettings } from "../sessions/store";
import { useUiPreferences } from "../ui/store";

type NewChatPatch = Partial<Omit<ThreadSettings, "goal">>;

export async function createInteractiveChat(
  settings?: NewChatPatch,
  options: { forceWorktree?: boolean } = {},
): Promise<string> {
  const sessions = useSessions.getState();
  const folder = (settings?.folder ?? sessions.getSettings(sessions.activeId).folder).trim();
  const policy = options.forceWorktree ? "worktree" : useUiPreferences.getState().newProjectChatWorkspace;
  const id = sessions.newUserChat(settings);
  if (!folder || policy === "current") return id;
  try {
    await setWorkspace(folder);
    const status = await getWorkspaceGitStatus();
    if (!status || status.state !== "ready" || !status.is_repo) return id;
    if (policy === "ask" && !window.confirm("Create this project chat in an isolated worktree?")) return id;
    const result = await runWorkspaceGitAction("create_thread_worktree", { thread_id: id });
    if (!result.ok || !result.worktree) throw new Error(result.message || "Could not create the isolated worktree.");
    useSessions.getState().setThreadWorkspace(id, {
      mode: "worktree",
      projectFolder: folder,
      worktreeFolder: result.worktree,
      branch: result.stdout.trim() || undefined,
      baseCommit: result.head,
      createdAt: Date.now(),
    });
    await setWorkspace(result.worktree);
  } catch (error) {
    useUiPreferences.getState().pushNotice({
      tone: "error",
      message: `The chat stayed in the current checkout because worktree creation failed: ${error instanceof Error ? error.message : String(error)} Retry from the project's isolated-chat action.`,
    });
  }
  return id;
}
