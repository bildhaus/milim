import {
  useEffect,
  useRef,
  type Dispatch,
  type SetStateAction,
} from "react";
import {
  getMobileCompanionStatus,
  pollMobileCompanionEvents,
  publishMobileThreadSnapshot,
  runWorkspaceGitAction,
  setWorkspace,
  type ChatAttachment,
  type ChatMessage,
  type MobileRelayAttachment,
  type MobileRelayEvent,
} from "../../api";
import { useAgents } from "../../agents/store";
import { assertValidImageAttachment } from "../../lib/attachmentInput";
import { nativeRuntimeIsStale } from "../../lib/hotSwap";
import { appendUserTurn } from "../../lib/turnContext";
import { queuedModelForSession } from "../../lib/turnQueue";
import { useSessions } from "../../sessions/store";

type MobileSnapshot = Parameters<typeof publishMobileThreadSnapshot>[0];

function documentVisible(): boolean {
  return typeof document === "undefined" || document.visibilityState === "visible";
}

export function useChatMobileRelayController({
  pollKey,
  snapshot,
  setInput,
  setPendingAttachments,
  setChatNotice,
  setProvidersOpen,
  pushNotice,
  createAttachmentId,
  runTurn,
  runTurnAndDrain,
  drainQueuedMessages,
  approveWorkerRun,
  continueWorkerRunSolo,
  stopActiveWorkerRun,
  stop,
  regenerate,
  deleteMessageAt,
}: {
  pollKey: string;
  snapshot: MobileSnapshot;
  setInput: Dispatch<SetStateAction<string>>;
  setPendingAttachments: Dispatch<SetStateAction<ChatAttachment[]>>;
  setChatNotice: (notice: MobileChatNotice | null) => void;
  setProvidersOpen: (open: boolean) => void;
  pushNotice: (notice: MobileChatNotice) => void;
  createAttachmentId: () => string;
  runTurn: (
    messages: ChatMessage[],
    model: string,
    options: Record<string, never>,
    sessionId: string,
  ) => Promise<{ status: string }>;
  runTurnAndDrain: (
    messages: ChatMessage[],
    model: string,
  ) => Promise<unknown>;
  drainQueuedMessages: (sessionId: string, model: string) => Promise<unknown>;
  approveWorkerRun: (runId: string) => Promise<void>;
  continueWorkerRunSolo: (runId: string) => Promise<void>;
  stopActiveWorkerRun: (runId: string) => Promise<void>;
  stop: () => void;
  regenerate: () => void;
  deleteMessageAt: (index: number) => void;
}) {
  const activeId = useSessions((state) => state.activeId);
  const activeSession = useSessions((state) =>
    state.sessions.find((session) => session.id === state.activeId),
  );
  const activeWorkerRunId = useSessions((state) =>
    state.workerRuns.find(
      (record) => record.run.parent_thread_id === state.activeId,
    )?.run.id,
  );
  const busy = useSessions((state) =>
    state.generatingSessionIds.includes(state.activeId),
  );
  const agents = useAgents((state) => state.agents);
  const effectiveModel =
    activeSession?.worker?.model || activeSession?.settings?.model || "";
  const messages = activeSession?.messages ?? [];
  const threadSettings = activeSession?.settings;
  const applyMobileRelayEventRef = useRef<(event: MobileRelayEvent) => void>(
    () => {},
  );
  const mobileRelayPollingRef = useRef(false);
  const mobileRelayReadyRef = useRef(false);

  function mobileRelayAttachments(
    attachments?: MobileRelayAttachment[],
  ): ChatAttachment[] {
    return (attachments ?? [])
      .filter(
        (attachment) =>
          attachment.name && attachment.mime && attachment.size >= 0,
      )
      .flatMap((attachment) => {
        const next = {
          id: attachment.id || createAttachmentId(),
          name: attachment.name,
          mime: attachment.mime,
          size: attachment.size,
          content: attachment.content,
          dataUrl: attachment.dataUrl,
          truncated: Boolean(attachment.truncated),
        };
        try {
          assertValidImageAttachment(next);
          return [next];
        } catch (error) {
          setChatNotice({
            tone: "error",
            message: error instanceof Error ? error.message : String(error),
          });
          return [];
        }
      });
  }

  function appendMobileRelayText(
    text: string,
    attachments: ChatAttachment[] = [],
  ) {
    if (text) {
      setInput((current) => {
        const trimmed = current.trimEnd();
        return trimmed ? `${trimmed}\n${text}` : text;
      });
    }
    if (attachments.length) {
      setPendingAttachments((current) =>
        [...current, ...attachments].slice(0, 12),
      );
    }
  }

  function sendMobileRelayText(event: MobileRelayEvent) {
    const text = event.text.trim();
    const attachments = mobileRelayAttachments(event.attachments);
    if (!text && attachments.length === 0) return;
    if (busy) {
      useSessions
        .getState()
        .enqueueQueuedMessage(activeId, { content: text, attachments });
      setChatNotice({
        tone: "info",
        message: `Mobile relay from ${event.device_name} queued.`,
      });
      return;
    }
    const selectedModel = effectiveModel.trim();
    if (!selectedModel) {
      appendMobileRelayText(text, attachments);
      setProvidersOpen(true);
      setChatNotice({
        tone: "error",
        message:
          "Mobile relay is waiting in the composer. Choose a model before sending.",
      });
      return;
    }
    setInput("");
    setPendingAttachments([]);
    setChatNotice({
      tone: "info",
      message: `Mobile relay from ${event.device_name} sent.`,
    });
    void runTurnAndDrain(
      appendUserTurn(
        messages,
        text,
        attachments.length ? attachments : undefined,
      ),
      selectedModel,
    );
  }

  function startMobileThread(event: MobileRelayEvent) {
    const text = event.text.trim();
    const attachments = mobileRelayAttachments(event.attachments);
    const store = useSessions.getState();
    store.newChat(threadSettings);
    const sessionId = useSessions.getState().activeId;
    setInput("");
    setPendingAttachments([]);
    if (!text && attachments.length === 0) {
      setChatNotice({
        tone: "info",
        message: `Mobile relay from ${event.device_name} created a thread.`,
      });
      return;
    }
    const selectedModel = queuedModelForSession(
      sessionId,
      effectiveModel.trim(),
      agents,
    );
    if (!selectedModel) {
      useSessions
        .getState()
        .enqueueQueuedMessage(sessionId, { content: text, attachments });
      setProvidersOpen(true);
      setChatNotice({
        tone: "error",
        message:
          "Mobile relay created a thread, but a model is needed before sending.",
      });
      return;
    }
    setChatNotice({
      tone: "info",
      message: `Mobile relay from ${event.device_name} started a thread.`,
    });
    void runTurn(
      appendUserTurn([], text, attachments.length ? attachments : undefined),
      selectedModel,
      {},
      sessionId,
    ).then((result) => {
      if (result.status === "done")
        void drainQueuedMessages(sessionId, selectedModel);
    });
  }

  function activeOrPayloadThreadId(text: string): string {
    return text.trim() || activeId;
  }

  async function deleteThreadWithRetryCleanup(sessionId: string) {
    const session = useSessions
      .getState()
      .sessions.find((item) => item.id === sessionId);
    const isolated =
      session?.threadWorkspace?.mode === "worktree"
        ? session.threadWorkspace
        : null;
    if (isolated) {
      try {
        await setWorkspace(isolated.projectFolder);
        let result = await runWorkspaceGitAction("remove_thread_worktree", {
          thread_id: sessionId,
        });
        if (!result.ok && result.message.includes("uncommitted changes")) {
          const discard = window.confirm(
            `${result.message}\n\nForce-discard the worktree? The branch will be retained.`,
          );
          if (!discard) return;
          result = await runWorkspaceGitAction("remove_thread_worktree", {
            thread_id: sessionId,
            force: true,
          });
        }
        if (!result.ok) throw new Error(result.message);
      } catch (error) {
        setChatNotice({
          tone: "error",
          message: `Thread was kept because worktree cleanup failed: ${
            error instanceof Error ? error.message : String(error)
          }`,
        });
        return;
      }
    }
    const retry = session?.retryWorkspace;
    if (retry) {
      try {
        await setWorkspace(retry.originalFolder);
        const result = await runWorkspaceGitAction("remove_retry_worktree", {
          worktree: retry.worktreeFolder,
        });
        if (!result.ok) throw new Error(result.message);
      } catch (error) {
        setChatNotice({
          tone: "error",
          message: `Retry thread was kept because cleanup failed: ${
            error instanceof Error ? error.message : String(error)
          }`,
        });
        return;
      }
    }
    useSessions.getState().remove(sessionId);
  }

  function applyMobileRelayEvent(event: MobileRelayEvent) {
    if (event.action === "new_thread") {
      startMobileThread(event);
      return;
    }
    if (event.action === "worker_run_start") {
      const runId = event.text.trim() || activeWorkerRunId;
      if (runId) void approveWorkerRun(runId);
      return;
    }
    if (event.action === "worker_run_continue_solo") {
      const runId = event.text.trim() || activeWorkerRunId;
      if (runId) void continueWorkerRunSolo(runId);
      return;
    }
    if (event.action === "worker_run_stop") {
      const runId = event.text.trim() || activeWorkerRunId;
      if (runId) void stopActiveWorkerRun(runId);
      return;
    }
    if (event.action === "stop") {
      stop();
      setChatNotice({
        tone: "info",
        message: `Mobile relay from ${event.device_name} stopped generation.`,
      });
      return;
    }
    if (event.action === "regenerate") {
      regenerate();
      return;
    }
    if (event.action === "delete_message") {
      const index = Number.parseInt(event.text.trim(), 10);
      if (Number.isFinite(index)) deleteMessageAt(index);
      return;
    }
    if (event.action === "rename_thread") {
      const title = event.text.trim();
      if (title) useSessions.getState().rename(activeId, title);
      return;
    }
    if (event.action === "archive_thread") {
      useSessions
        .getState()
        .archiveSession(activeOrPayloadThreadId(event.text));
      return;
    }
    if (event.action === "delete_thread") {
      void deleteThreadWithRetryCleanup(activeOrPayloadThreadId(event.text));
      return;
    }
    if (event.action === "set_model") {
      const nextModel = event.text.trim();
      if (nextModel) {
        const session = useSessions
          .getState()
          .sessions.find((item) => item.id === activeId);
        const kind = nextModel.startsWith("codex:")
          ? "codex"
          : nextModel.startsWith("claude:")
            ? "claude"
            : nextModel.startsWith("opencode:")
              ? "opencode"
              : nextModel.startsWith("pi:")
                ? "pi"
                : null;
        if (session && kind && nativeRuntimeIsStale(session, kind)) {
          useSessions.getState().clearAccountRuntimeKind(activeId, kind);
        }
        useSessions.getState().updateSettings(activeId, { model: nextModel });
      }
      return;
    }
    if (event.action === "attach") {
      appendMobileRelayText(
        event.text.trim(),
        mobileRelayAttachments(event.attachments),
      );
      setChatNotice({
        tone: "info",
        message: `Mobile relay from ${event.device_name} added attachments.`,
      });
      return;
    }
    if (event.action === "switch_thread") {
      const targetId = event.text.trim();
      const target = useSessions
        .getState()
        .sessions.find(
          (session) => session.id === targetId && !session.archivedAt,
        );
      if (target) {
        useSessions.getState().switchTo(target.id);
        setChatNotice({
          tone: "info",
          message: `Mobile relay switched to ${target.title || "thread"}.`,
        });
      }
      return;
    }
    if (event.action === "replace") {
      setInput(event.text);
      setPendingAttachments(mobileRelayAttachments(event.attachments));
      setChatNotice({
        tone: "info",
        message: `Mobile relay from ${event.device_name} replaced the composer.`,
      });
      return;
    }
    if (event.action === "send") {
      sendMobileRelayText(event);
      return;
    }
    appendMobileRelayText(
      event.text,
      mobileRelayAttachments(event.attachments),
    );
    const notice = {
      tone: "info",
      message: `Mobile relay from ${event.device_name} added to the composer.`,
    } as const;
    setChatNotice(notice);
    pushNotice(notice);
  }
  applyMobileRelayEventRef.current = applyMobileRelayEvent;

  useEffect(() => {
    let cancelled = false;
    async function pollMobileRelay() {
      if (!documentVisible() || !mobileRelayReadyRef.current) return;
      if (mobileRelayPollingRef.current) return;
      mobileRelayPollingRef.current = true;
      try {
        const events = await pollMobileCompanionEvents();
        if (!cancelled) {
          for (const event of events)
            applyMobileRelayEventRef.current(event);
        }
      } catch {
        // The bridge is disabled in normal web previews and before pairing.
      } finally {
        mobileRelayPollingRef.current = false;
      }
    }
    async function refreshMobileRelayReady() {
      if (!documentVisible()) return;
      try {
        const status = await getMobileCompanionStatus();
        mobileRelayReadyRef.current = status.enabled && status.devices.length > 0;
        if (mobileRelayReadyRef.current) void pollMobileRelay();
      } catch {
        mobileRelayReadyRef.current = false;
      }
    }
    void refreshMobileRelayReady();
    const timer = window.setInterval(() => void pollMobileRelay(), 1500);
    const statusTimer = window.setInterval(
      () => void refreshMobileRelayReady(),
      30_000,
    );
    const onVisible = () => {
      if (documentVisible()) void refreshMobileRelayReady();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      window.clearInterval(statusTimer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [pollKey]);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void publishMobileThreadSnapshot(snapshot).catch(() => {});
    }, 300);
    return () => window.clearTimeout(timer);
  }, [snapshot]);
}

type MobileChatNotice = {
  message: string;
  tone: "info" | "warning" | "error";
};
