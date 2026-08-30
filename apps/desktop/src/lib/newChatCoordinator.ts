import {
  createControlCommandId,
  getWorkspaceGitStatus,
  runWorkspaceGitAction,
  sendControlCommand,
  setWorkspace,
  type ControlCommandV1,
} from "../api.js";
import { flushDeferredUserStateWrites } from "../persistence/userStateStorage.js";
import { useSessions, type ThreadSettingsPatch } from "../sessions/store.js";
import { useUiPreferences } from "../ui/store.js";
import { confirmApp } from "../ui/confirmation.js";
import { requestWorkspaceEditorLeave } from "./workspaceEditorGuard.js";

async function sendCreateCommand(command: ControlCommandV1) {
  try {
    return await sendControlCommand(command);
  } catch {
    return sendControlCommand(command);
  }
}

async function provisionCanonicalChat(settings?: ThreadSettingsPatch): Promise<string | null> {
  const sessions = useSessions.getState();
  const current = sessions.sessions.find((session) => session.id === sessions.activeId);
  const id = current?.messages.length === 0 ? current.id : crypto.randomUUID();
  const resolvedSettings = sessions.getNewUserChatSettings(settings);
  try {
    const result = await sendCreateCommand({
      command_id: createControlCommandId(),
      kind: "thread.create",
      thread_id: id,
      payload: { id, title: "New chat", settings: resolvedSettings },
    });
    if (result.status !== "applied") {
      throw new Error(result.message || `Thread creation returned ${result.status}.`);
    }
  } catch (error) {
    useUiPreferences.getState().pushNotice({
      tone: "error",
      message: `Milim could not create the chat: ${error instanceof Error ? error.message : String(error)}`,
    });
    return null;
  }
  const localId = useSessions.getState().newChat(resolvedSettings, id);
  try {
    await flushDeferredUserStateWrites("milim.sessions");
  } catch (error) {
    useUiPreferences.getState().pushNotice({
      tone: "error",
      message: `The chat was created, but Milim could not persist its local replica yet: ${error instanceof Error ? error.message : String(error)}`,
    });
  }
  return localId;
}

export async function createCanonicalChat(settings?: ThreadSettingsPatch): Promise<string> {
  return (await provisionCanonicalChat(settings)) ?? useSessions.getState().activeId;
}

export async function createInteractiveChat(
  settings?: ThreadSettingsPatch,
  options: { workspace?: "current" | "worktree" } = {},
): Promise<string> {
  const sessions = useSessions.getState();
  if (!(await requestWorkspaceEditorLeave("navigate"))) return sessions.activeId;
  const folder = (settings?.folder ?? sessions.getSettings(sessions.activeId).folder).trim();
  const policy = options.workspace ?? useUiPreferences.getState().newProjectChatWorkspace;
  const id = await provisionCanonicalChat(settings);
  if (!id) return useSessions.getState().activeId;
  if (!folder || policy === "current") return id;
  try {
    await setWorkspace(folder);
    const status = await getWorkspaceGitStatus();
    if (!status || status.state !== "ready" || !status.is_repo) return id;
    if (policy === "ask" && !(await confirmApp({
      title: "Use an isolated worktree?",
      message: "Create this project chat in an isolated worktree?",
      confirmLabel: "Create worktree",
    }))) return id;
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
