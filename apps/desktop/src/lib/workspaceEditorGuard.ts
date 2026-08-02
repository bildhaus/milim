export type WorkspaceEditorLeaveReason = "navigate" | "hide" | "quit";

type WorkspaceEditorGuard = (reason: WorkspaceEditorLeaveReason) => Promise<boolean>;

let activeGuard: WorkspaceEditorGuard | null = null;

export function registerWorkspaceEditorGuard(guard: WorkspaceEditorGuard): () => void {
  activeGuard = guard;
  return () => {
    if (activeGuard === guard) activeGuard = null;
  };
}

export async function requestWorkspaceEditorLeave(reason: WorkspaceEditorLeaveReason = "navigate"): Promise<boolean> {
  return activeGuard ? activeGuard(reason) : true;
}
