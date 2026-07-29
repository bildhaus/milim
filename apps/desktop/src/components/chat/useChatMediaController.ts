import { useEffect, useMemo, useState, type MutableRefObject } from "react";
import {
  generateMedia,
  getMediaStatus,
  getMediaModelSchema,
  listMediaModels,
  mediaProviders,
  type ChatAttachment,
  type ChatMessage,
  type MediaGenerationResult,
  type MediaKind,
  type MediaModelSchema,
  type MediaSchemaControl,
  type ModelInfo,
  type ProviderInfo,
} from "../../api";
import {
  bestMediaResultUrl,
  defaultMediaAdvanced,
  defaultMediaModel,
  inputWithSchemaControls,
  mediaKindForModelId,
  mediaPollingMaxAttempts,
  mediaPreferenceKey,
  mediaResultContent,
  parseControlValue,
  schemaDefaults,
  shouldPollMediaStatus,
} from "../../lib/media";
import { useSessions } from "../../sessions/store";
import { useSettings, type MediaSettings } from "../../settings/store";
import { useUiPreferences } from "../../ui/store";

const EMPTY_MESSAGES: ChatMessage[] = [];
const MEDIA_CONTEXT_MESSAGE_LIMIT = 10;
const MEDIA_CONTEXT_CHAR_LIMIT = 1800;

type MediaProviderCatalog = Record<
  string,
  Partial<Record<MediaKind, string[]>>
>;

export type ActiveMediaTarget = {
  provider: ProviderInfo;
  model: string;
  kind: MediaKind;
  supportedKinds: MediaKind[];
};

type MediaChatNotice = {
  message: string;
  tone: "info" | "warning" | "error";
};

function updateMediaMessage(
  sessionId: string,
  requestId: string,
  patch: Partial<ChatMessage>,
): void {
  const store = useSessions.getState();
  const session = store.sessions.find((item) => item.id === sessionId);
  if (!session) return;
  const messages = session.messages.map((message) =>
    message.mediaRequestId === requestId ? { ...message, ...patch } : message,
  );
  store.setMessages(sessionId, messages, { autoTitle: false });
}

function replaceMediaResult(
  sessionId: string,
  requestId: string,
  result: MediaGenerationResult,
): void {
  const store = useSessions.getState();
  const session = store.sessions.find((item) => item.id === sessionId);
  if (!session) return;
  const messages = session.messages.map((message) => {
    if (message.mediaRequestId !== requestId) return message;
    const current = message.mediaResults ?? [];
    const index = current.findIndex(
      (item) =>
        item.provider_id === result.provider_id && item.id === result.id,
    );
    const mediaResults =
      index >= 0
        ? current.map((item, itemIndex) =>
            itemIndex === index ? { ...item, ...result } : item,
          )
        : [result, ...current];
    return {
      ...message,
      content: bestMediaResultUrl(result) ? "" : mediaResultContent(result),
      mediaResults,
    };
  });
  store.setMessages(sessionId, messages, { autoTitle: false });
}

function compactMediaText(value: string, max = 96): string {
  const text = value.trim().replace(/\s+/g, " ");
  return text.length > max ? text.slice(0, max - 1).trimEnd() + "..." : text;
}

function mediaContextLine(message: ChatMessage): string | null {
  const text = message.content.trim();
  if (message.role === "user") {
    return text ? `User: ${compactMediaText(text, 420)}` : null;
  }
  if (message.mediaResults?.length) {
    const summaries = message.mediaResults.slice(0, 2).map((result) => {
      const kind = String(result.media[0]?.kind ?? result.kind ?? "media");
      const status = result.status.trim() || "submitted";
      return bestMediaResultUrl(result)
        ? `generated ${kind} (${status})`
        : `${kind} generation ${status}`;
    });
    return summaries.length ? `Assistant: ${summaries.join(", ")}.` : null;
  }
  if (!text || text.startsWith("Generating ")) return null;
  return `Assistant: ${compactMediaText(text, 420)}`;
}

function mediaPromptWithHistory(
  baseMessages: ChatMessage[],
  currentPrompt: string,
): string {
  const lines = baseMessages
    .map(mediaContextLine)
    .filter((line): line is string => Boolean(line))
    .slice(-MEDIA_CONTEXT_MESSAGE_LIMIT);
  const selected: string[] = [];
  let chars = 0;
  for (const line of lines.slice().reverse()) {
    const nextChars = chars + line.length + 1;
    if (selected.length && nextChars > MEDIA_CONTEXT_CHAR_LIMIT) break;
    selected.push(line);
    chars = nextChars;
  }
  const context = selected.reverse();
  if (!context.length) return currentPrompt;
  return [
    "Use the recent chat context only to resolve references and maintain continuity. Create the latest requested media.",
    "",
    "Recent chat:",
    ...context,
    "",
    `Latest request: ${currentPrompt}`,
  ].join("\n");
}

function addMediaCandidate(
  candidates: Map<string, Set<MediaKind>>,
  model: string | undefined,
  fallbackKind: MediaKind,
  force = false,
): void {
  const trimmed = model?.trim();
  if (!trimmed) return;
  const kind = mediaKindForModelId(trimmed);
  if (!kind && !force) return;
  const kinds = candidates.get(trimmed) ?? new Set<MediaKind>();
  kinds.add(kind ?? fallbackKind);
  candidates.set(trimmed, kinds);
}

function mediaCandidatesForProvider(
  provider: ProviderInfo,
  settings: MediaSettings,
  catalog: MediaProviderCatalog,
): Map<string, Set<MediaKind>> {
  const candidates = new Map<string, Set<MediaKind>>();
  addMediaCandidate(candidates, defaultMediaModel(provider), "image", true);
  addMediaCandidate(candidates, settings.modelByProvider[provider.id], "image", true);
  for (const id of settings.favoriteModelIdsByProvider[provider.id] ?? []) {
    addMediaCandidate(candidates, id, "image", true);
  }
  for (const id of provider.models ?? []) {
    addMediaCandidate(candidates, id, "image");
  }
  for (const kind of ["image", "video", "music"] as MediaKind[]) {
    for (const id of catalog[provider.id]?.[kind] ?? []) {
      addMediaCandidate(candidates, id, kind, true);
    }
  }
  return candidates;
}

function mediaModelsForPicker(
  providers: ProviderInfo[],
  settings: MediaSettings,
  catalog: MediaProviderCatalog,
): ModelInfo[] {
  return providers.flatMap((provider) =>
    Array.from(
      mediaCandidatesForProvider(provider, settings, catalog),
      ([id, kinds]) => ({
        id,
        owned_by: `${provider.name} media`,
        capabilities: {
          imageOutput: kinds.has("image"),
          videoOutput: kinds.has("video"),
          musicOutput: kinds.has("music"),
        },
      }),
    ),
  );
}

function resolveActiveMediaTarget(
  model: string,
  providers: ProviderInfo[],
  settings: MediaSettings,
  catalog: MediaProviderCatalog,
): ActiveMediaTarget | null {
  const selected = model.trim();
  if (!selected) return null;
  for (const provider of providers) {
    const kinds = mediaCandidatesForProvider(provider, settings, catalog).get(selected);
    if (kinds?.size) {
      const supportedKinds = Array.from(kinds);
      return {
        provider,
        model: selected,
        kind: supportedKinds[0],
        supportedKinds,
      };
    }
  }
  return null;
}

export function useChatMediaController({
  providers,
  effectiveModel,
  mediaSettings,
  setMediaSettings,
  pendingAttachments,
  setInput,
  setPendingAttachments,
  setChatNotice,
  generationControllersRef,
  createRequestId,
}: {
  providers: ProviderInfo[];
  effectiveModel: string;
  mediaSettings: MediaSettings;
  setMediaSettings: (patch: Partial<MediaSettings>) => void;
  pendingAttachments: ChatAttachment[];
  setInput: (value: string) => void;
  setPendingAttachments: (attachments: ChatAttachment[]) => void;
  setChatNotice: (notice: MediaChatNotice | null) => void;
  generationControllersRef: MutableRefObject<Map<string, AbortController>>;
  createRequestId: () => string;
}) {
  const activeId = useSessions((state) => state.activeId);
  const messages = useSessions(
    (state) =>
      state.sessions.find((session) => session.id === state.activeId)?.messages ??
      EMPTY_MESSAGES,
  );
  const setMessages = useSessions((state) => state.setMessages);
  const autoTitleChats = useUiPreferences((state) => state.autoTitleChats);
  const [mediaCatalog, setMediaCatalog] = useState<MediaProviderCatalog>({});
  const [mediaKind, setMediaKind] = useState<MediaKind>("image");
  const [mediaAdvanced, setMediaAdvanced] = useState("{}");
  const [mediaSchema, setMediaSchema] = useState<MediaModelSchema | null>(null);
  const [mediaParameterValues, setMediaParameterValues] = useState<Record<string, unknown>>({});
  const [mediaSchemaLoading, setMediaSchemaLoading] = useState(false);
  const [mediaError, setMediaError] = useState<string | null>(null);
  const enabledMediaProviders = useMemo(() => mediaProviders(providers), [providers]);
  const mediaModelEntries = useMemo(
    () => mediaModelsForPicker(enabledMediaProviders, mediaSettings, mediaCatalog),
    [enabledMediaProviders, mediaCatalog, mediaSettings],
  );
  const activeMediaTarget = useMemo(
    () =>
      resolveActiveMediaTarget(
        effectiveModel,
        enabledMediaProviders,
        mediaSettings,
        mediaCatalog,
      ),
    [effectiveModel, enabledMediaProviders, mediaCatalog, mediaSettings],
  );

  useEffect(() => {
    if (enabledMediaProviders.length === 0) {
      setMediaCatalog({});
      return;
    }
    let cancelled = false;
    async function loadMediaCatalogs() {
      const rows = await Promise.all(
        enabledMediaProviders.flatMap((provider) =>
          (["image", "video", "music"] as MediaKind[]).map(async (kind) => {
            try {
              const models = await listMediaModels(provider.id, kind);
              return {
                providerId: provider.id,
                kind,
                ids: models.map((item) => item.id).filter(Boolean),
              };
            } catch {
              return { providerId: provider.id, kind, ids: [] };
            }
          }),
        ),
      );
      if (cancelled) return;
      const next: MediaProviderCatalog = {};
      for (const row of rows) {
        next[row.providerId] ??= {};
        next[row.providerId][row.kind] = row.ids;
      }
      setMediaCatalog(next);
    }
    void loadMediaCatalogs();
    return () => {
      cancelled = true;
    };
  }, [enabledMediaProviders.map((provider) => provider.id).join("\u0000")]);

  useEffect(() => {
    if (!activeMediaTarget) {
      setMediaSchema(null);
      setMediaParameterValues({});
      setMediaSchemaLoading(false);
      setMediaError(null);
      return;
    }
    const key = mediaPreferenceKey(
      activeMediaTarget.provider.id,
      activeMediaTarget.model,
    );
    const saved = useSettings.getState().media;
    setMediaKind(activeMediaTarget.kind);
    setMediaAdvanced(
      saved.advancedByProviderModel[key] ??
        defaultMediaAdvanced(activeMediaTarget.provider),
    );
    setMediaParameterValues(saved.parametersByProviderModel[key] ?? {});
    setMediaSettings({
      providerId: activeMediaTarget.provider.id,
      modelByProvider: {
        ...saved.modelByProvider,
        [activeMediaTarget.provider.id]: activeMediaTarget.model,
      },
    });
  }, [activeMediaTarget?.provider.id, activeMediaTarget?.model]);

  useEffect(() => {
    if (!activeMediaTarget) {
      setMediaSchema(null);
      setMediaSchemaLoading(false);
      return;
    }
    const kind = activeMediaTarget.supportedKinds.includes(mediaKind)
      ? mediaKind
      : activeMediaTarget.kind;
    const key = mediaPreferenceKey(
      activeMediaTarget.provider.id,
      activeMediaTarget.model,
    );
    let cancelled = false;
    setMediaSchema(null);
    setMediaSchemaLoading(true);
    setMediaError(null);
    void getMediaModelSchema(
      activeMediaTarget.provider.id,
      activeMediaTarget.model,
      kind,
    )
      .then((schema) => {
        if (cancelled) return;
        setMediaSchema(schema);
        const nextSaved =
          useSettings.getState().media.parametersByProviderModel[key];
        setMediaParameterValues({ ...schemaDefaults(schema), ...nextSaved });
      })
      .catch((error) => {
        if (!cancelled) {
          setMediaSchema(null);
          setMediaError(error instanceof Error ? error.message : String(error));
        }
      })
      .finally(() => {
        if (!cancelled) setMediaSchemaLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [activeMediaTarget?.provider.id, activeMediaTarget?.model, mediaKind]);

  function updateInlineMediaAdvanced(value: string) {
    setMediaAdvanced(value);
    if (!activeMediaTarget) return;
    const key = mediaPreferenceKey(
      activeMediaTarget.provider.id,
      activeMediaTarget.model,
    );
    const saved = useSettings.getState().media;
    setMediaSettings({
      advancedByProviderModel: {
        ...saved.advancedByProviderModel,
        [key]: value,
      },
    });
  }

  function updateInlineMediaParameter(
    control: MediaSchemaControl,
    value: string | boolean,
  ) {
    if (!activeMediaTarget) return;
    let parsed: unknown;
    try {
      parsed =
        typeof value === "string" &&
        (control.kind === "array" || control.kind === "json")
          ? value
          : parseControlValue(control, value);
    } catch (error) {
      setMediaError(error instanceof Error ? error.message : String(error));
      return;
    }
    const next = { ...mediaParameterValues, [control.key]: parsed };
    const key = mediaPreferenceKey(
      activeMediaTarget.provider.id,
      activeMediaTarget.model,
    );
    const saved = useSettings.getState().media;
    setMediaParameterValues(next);
    setMediaSettings({
      parametersByProviderModel: {
        ...saved.parametersByProviderModel,
        [key]: next,
      },
    });
    setMediaError(null);
  }

  function updateInlineMediaKind(kind: MediaKind) {
    setMediaSchema(null);
    setMediaSchemaLoading(true);
    setMediaKind(kind);
  }

  async function pollInlineMediaStatus(
    initial: MediaGenerationResult,
    sessionId: string,
    requestId: string,
  ) {
    if (!shouldPollMediaStatus(initial)) return;
    let current = initial;
    try {
      for (
        let attempt = 0;
        attempt < mediaPollingMaxAttempts(initial);
        attempt += 1
      ) {
        await new Promise((resolve) => window.setTimeout(resolve, 3000));
        const next = await getMediaStatus({
          provider_id: current.provider_id,
          id: current.id,
          model: current.model,
          response_url: current.urls.response,
          status_url: current.urls.status,
          kind: current.kind as MediaKind,
        });
        current = next;
        replaceMediaResult(sessionId, requestId, next);
        if (!shouldPollMediaStatus(next) || next.media.length > 0) break;
      }
    } catch (error) {
      updateMediaMessage(sessionId, requestId, {
        content: `Media status failed: ${
          error instanceof Error ? error.message : String(error)
        }`,
      });
    }
  }

  async function sendMediaPrompt(
    text: string,
    target: ActiveMediaTarget,
    baseMessages: ChatMessage[] = messages,
    checkPendingAttachments = true,
  ) {
    if (checkPendingAttachments && pendingAttachments.length > 0) {
      setChatNotice({
        tone: "error",
        message:
          "Media generation uses the prompt text only. Remove attachments or choose a chat model.",
      });
      return;
    }
    if (generationControllersRef.current.has(activeId)) return;
    if (
      mediaSchemaLoading ||
      !mediaSchema ||
      mediaSchema.provider_id !== target.provider.id ||
      mediaSchema.model !== target.model
    ) {
      const message = "Media settings are still loading. Try again in a moment.";
      setMediaError(message);
      setChatNotice({ tone: "error", message });
      return;
    }

    let requestInput: Record<string, unknown>;
    try {
      requestInput = inputWithSchemaControls(
        mediaAdvanced,
        mediaSchema,
        mediaParameterValues,
      );
    } catch (error) {
      const message =
        error instanceof Error ? error.message : String(error);
      setMediaError(message);
      setChatNotice({ tone: "error", message });
      return;
    }

    const sessionId = activeId;
    const requestId = createRequestId();
    const kind = target.supportedKinds.includes(mediaKind)
      ? mediaKind
      : target.kind;
    const prompt = mediaPromptWithHistory(baseMessages, text);
    const userMessage: ChatMessage = { role: "user", content: text };
    const assistantMessage: ChatMessage = {
      role: "assistant",
      content: `Generating ${kind} with ${target.model}...`,
      mediaRequestId: requestId,
    };
    setInput("");
    setPendingAttachments([]);
    setChatNotice(null);
    setMediaError(null);
    setMessages(sessionId, [...baseMessages, userMessage, assistantMessage], {
      autoTitle: autoTitleChats,
    });

    const store = useSessions.getState();
    const controller = new AbortController();
    generationControllersRef.current.set(sessionId, controller);
    store.setSessionGenerating(sessionId, true);
    try {
      const result = await generateMedia(
        {
          provider_id: target.provider.id,
          kind,
          model: target.model,
          prompt,
          input: requestInput,
        },
        controller.signal,
      );
      replaceMediaResult(sessionId, requestId, result);
      void pollInlineMediaStatus(result, sessionId, requestId);
    } catch (error) {
      const aborted = error instanceof DOMException && error.name === "AbortError";
      updateMediaMessage(sessionId, requestId, {
        content: aborted
          ? "Media generation stopped."
          : `Media generation failed: ${
              error instanceof Error ? error.message : String(error)
            }`,
      });
    } finally {
      generationControllersRef.current.delete(sessionId);
      store.setSessionGenerating(sessionId, false);
      store.setSessionUnread(
        sessionId,
        useSessions.getState().activeId !== sessionId,
      );
    }
  }

  return {
    activeMediaTarget,
    enabledMediaProviders,
    mediaAdvanced,
    mediaCatalog,
    mediaError,
    mediaKind,
    mediaModelEntries,
    mediaParameterValues,
    mediaSchema,
    mediaSchemaLoading,
    sendMediaPrompt,
    setMediaAdvanced,
    setMediaError,
    setMediaKind: updateInlineMediaKind,
    setMediaParameterValues,
    updateInlineMediaAdvanced,
    updateInlineMediaParameter,
  };
}
