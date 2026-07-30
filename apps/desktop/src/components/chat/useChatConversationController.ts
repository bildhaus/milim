import {
  useEffect,
  useRef,
  type Dispatch,
  type SetStateAction,
} from "react";
import { useAgents } from "../../agents/store";
import type { ChatMessage } from "../../api";
import {
  DEFAULT_GOAL_SETTINGS,
  normalizeGoalSettings,
  type GoalSettings,
} from "../../lib/goals";
import { sendMilimNotification } from "../../lib/nativeNotifications";
import {
  drainQueuedMessages as drainQueuedMessagesFromQueue,
} from "../../lib/turnQueue";
import { useSessions } from "../../sessions/store";
import { useUiPreferences } from "../../ui/store";
import { playInterfaceSound } from "../../ui/sounds";

export type GoalLoopState = {
  sessionId: string;
  stopped: boolean;
  decisionController?: AbortController;
};

export function useChatConversationController<
  TOptions extends object,
  TResult extends ConversationTurnResult,
>({
  queueInterrupts,
  setQueueInterrupts,
  generatingSessionIds,
  liveWorkerSessionIdsKey,
  setChatNotice,
  setGoalPanelOpen,
  setGoalPrefill,
  sessionMessages,
  runTurn,
}: {
  queueInterrupts: Record<string, string>;
  setQueueInterrupts: Dispatch<SetStateAction<Record<string, string>>>;
  generatingSessionIds: string[];
  liveWorkerSessionIdsKey: string;
  setChatNotice: (notice: ConversationNotice | null) => void;
  setGoalPanelOpen: (open: boolean) => void;
  setGoalPrefill: (prefill: string | null) => void;
  sessionMessages: (sessionId: string) => ChatMessage[];
  runTurn: (
    messages: ChatMessage[],
    model: string | undefined,
    options: TOptions,
    sessionId: string,
  ) => Promise<TResult>;
}) {
  const activeId = useSessions((state) => state.activeId);
  const generationControllersRef = useRef<Map<string, AbortController>>(new Map());
  const goalLoopRef = useRef<GoalLoopState | null>(null);
  const queueDrainRef = useRef<
    Map<string, Promise<ConversationTurnResult | undefined>>
  >(new Map());
  const compactionInFlightRef = useRef(false);
  const callbacksRef = useRef({ setChatNotice, sessionMessages, runTurn });
  callbacksRef.current = { setChatNotice, sessionMessages, runTurn };

  function sessionGoal(sessionId: string): GoalSettings {
    return useSessions.getState().getSettings(sessionId).goal;
  }

  function updateGoalState(
    sessionId: string,
    patch: Partial<GoalSettings>,
    baseGoal = sessionGoal(sessionId),
  ): GoalSettings {
    const next = normalizeGoalSettings({
      ...baseGoal,
      ...patch,
      updatedAt: Date.now(),
    });
    useSessions.getState().updateSettings(sessionId, { goal: next });
    return next;
  }

  function pauseGoalRun(reason = "Goal paused.", sessionId = activeId) {
    const loop = goalLoopRef.current;
    if (loop?.sessionId === sessionId) {
      loop.stopped = true;
      loop.decisionController?.abort();
    }
    generationControllersRef.current.get(sessionId)?.abort();
    const current = sessionGoal(sessionId);
    if (current.status === "running") {
      updateGoalState(
        sessionId,
        { status: "paused", lastReason: reason },
        current,
      );
    }
  }

  function draftToGoal(
    draft: GoalDraft,
    current: GoalSettings,
  ): GoalSettings {
    const contentChanged =
      draft.objective !== current.objective ||
      draft.successCriteria !== current.successCriteria ||
      draft.constraints !== current.constraints;
    const status = !draft.objective.trim()
      ? "idle"
      : current.status === "running"
        ? "paused"
        : contentChanged &&
            (current.status === "complete" ||
              current.status === "blocked" ||
              current.status === "error")
          ? "paused"
          : current.status;
    return normalizeGoalSettings({
      ...current,
      objective: draft.objective,
      successCriteria: draft.successCriteria,
      constraints: draft.constraints,
      developerMaxTurns: draft.developerMaxTurns,
      status,
      lastReason:
        current.status === "running"
          ? "Goal paused for edits."
          : current.lastReason,
      updatedAt: Date.now(),
    });
  }

  function saveGoalDraft(
    draft: GoalDraft,
    sessionId = activeId,
  ): GoalSettings {
    const current = sessionGoal(sessionId);
    if (current.status === "running")
      pauseGoalRun("Goal paused for edits.", sessionId);
    const next = draftToGoal(draft, current);
    useSessions.getState().updateSettings(sessionId, { goal: next });
    setGoalPrefill(null);
    return next;
  }

  function deleteGoal(sessionId = activeId) {
    pauseGoalRun("Goal deleted.", sessionId);
    useSessions
      .getState()
      .updateSettings(sessionId, { goal: DEFAULT_GOAL_SETTINGS });
    setGoalPrefill(null);
    setGoalPanelOpen(false);
  }

  function markGoalSeen(sessionId = activeId) {
    const current = sessionGoal(sessionId);
    const updatedAt = current.updatedAt ?? 0;
    if (!updatedAt || (current.lastSeenAt ?? 0) >= updatedAt) return;
    useSessions.getState().updateSettings(sessionId, {
      goal: { ...current, lastSeenAt: Date.now() },
    });
  }

  function openGoalPanel(prefill: string | null = null) {
    setGoalPrefill(prefill);
    markGoalSeen();
    setGoalPanelOpen(true);
    callbacksRef.current.setChatNotice(null);
  }

  async function drainQueuedMessages(
    sessionId: string,
    fallbackModel?: string,
  ) {
    return drainQueuedMessagesFromQueue({
      sessionId,
      fallbackModel,
      queueDrainRef,
      generationControllersRef,
      agents: useAgents.getState().agents,
      setChatNotice: callbacksRef.current.setChatNotice,
      sessionMessages: callbacksRef.current.sessionMessages,
      runTurn: (messages, selectedModel, targetSessionId) =>
        callbacksRef.current.runTurn(
          messages,
          selectedModel,
          {} as TOptions,
          targetSessionId,
        ),
    });
  }

  async function runTurnAndDrain(
    messages: ChatMessage[],
    selectedModel?: string,
    options: TOptions = {} as TOptions,
  ) {
    const sessionId = activeId;
    const result = await callbacksRef.current.runTurn(
      messages,
      selectedModel,
      options,
      sessionId,
    );
    let terminalResult: ConversationTurnResult = result;
    if (result.status === "done") {
      terminalResult =
        (await drainQueuedMessages(sessionId, selectedModel)) ?? result;
    }
    if (
      document.visibilityState === "visible" &&
      useSessions.getState().activeId === sessionId
    ) {
      const preferences = useUiPreferences.getState();
      if (
        preferences.interfaceSounds &&
        terminalResult.status === "done" &&
        preferences.soundOnFinished
      )
        playInterfaceSound(preferences.finishedSound);
      else if (
        preferences.interfaceSounds &&
        terminalResult.status === "error" &&
        preferences.soundOnAttention
      )
        playInterfaceSound(preferences.attentionSound);
    }
    const preferences = useUiPreferences.getState();
    const threadTitle = useSessions
      .getState()
      .sessions.find((session) => session.id === sessionId)?.title;
    if (terminalResult.status === "done" && preferences.notifyRunFinished) {
      void sendMilimNotification("finished", {
        threadTitle,
        includeThreadTitle: preferences.notificationIncludeThreadTitle,
        onlyWhenUnfocused: preferences.notifyOnlyWhenUnfocused,
      });
    } else if (
      terminalResult.status === "error" &&
      preferences.notifyNeedsAttention
    ) {
      void sendMilimNotification("attention", {
        threadTitle,
        includeThreadTitle: preferences.notificationIncludeThreadTitle,
        onlyWhenUnfocused: preferences.notifyOnlyWhenUnfocused,
      });
    }
    return result;
  }

  useEffect(() => {
    const interruptedSessionIds = Object.keys(queueInterrupts);
    if (interruptedSessionIds.length === 0) return;
    const generating = new Set(generatingSessionIds);
    const liveWorkers = new Set(
      liveWorkerSessionIdsKey ? liveWorkerSessionIdsKey.split("\0") : [],
    );
    const ready = interruptedSessionIds.filter(
      (sessionId) =>
        !generating.has(sessionId) && !liveWorkers.has(sessionId),
    );
    if (ready.length === 0) return;
    const activeDrains = new Set(
      ready.filter((sessionId) => queueDrainRef.current.has(sessionId)),
    );
    setQueueInterrupts((current) => {
      const next = { ...current };
      for (const sessionId of ready) delete next[sessionId];
      return next;
    });
    for (const sessionId of ready) {
      void drainQueuedMessages(sessionId).then(() => {
        if (activeDrains.has(sessionId)) void drainQueuedMessages(sessionId);
      });
    }
  }, [
    generatingSessionIds,
    liveWorkerSessionIdsKey,
    queueInterrupts,
    setQueueInterrupts,
  ]);

  useEffect(
    () => () => {
      const store = useSessions.getState();
      generationControllersRef.current.forEach((controller, id) => {
        controller.abort();
        store.setSessionGenerating(id, false);
      });
      generationControllersRef.current.clear();
      goalLoopRef.current?.decisionController?.abort();
    },
    [],
  );

  return {
    compactionInFlightRef,
    drainQueuedMessages,
    deleteGoal,
    generationControllersRef,
    goalLoopRef,
    openGoalPanel,
    pauseGoalRun,
    queueDrainRef,
    runTurnAndDrain,
    saveGoalDraft,
    sessionGoal,
    updateGoalState,
  };
}

type ConversationTurnResult = {
  status: "done" | "aborted" | "error" | "skipped";
  messages: ChatMessage[];
  error?: string;
};

type ConversationNotice = {
  message: string;
  tone: "info" | "warning" | "error";
};

type GoalDraft = {
  objective: string;
  successCriteria: string;
  constraints: string;
  developerMaxTurns: number | null;
};
