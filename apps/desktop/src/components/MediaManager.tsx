import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { createPortal } from "react-dom";
import {
  deleteMediaLibraryItem,
  generateMedia,
  getPrivacyMode,
  getMediaModelSchema,
  getMediaStatus,
  listMediaLibrary,
  listMediaModels,
  listProviders,
  mediaProviders,
  openArtifactLocation,
  openExternalUrl,
  refreshMediaLibraryItem,
  supportsMediaMetadataProvider,
  type MediaGenerationResult,
  type MediaKind,
  type MediaLibraryItem,
  type MediaLibraryStatus,
  type MediaModelInfo,
  type MediaModelSchema,
  type MediaSchemaControl,
  type ModelInfo,
  type ProviderInfo,
} from "../api";
import {
  DEFAULT_MEDIA_ADVANCED_INPUT,
  defaultMediaAdvanced,
  defaultMediaModel,
  inputWithSchemaControls,
  mediaPreferenceKey,
  mediaPollingMaxAttempts,
  parseControlValue,
  schemaDefaults,
  shouldPollMediaStatus,
  isTerminalMediaStatus,
} from "../lib/media";
import { useSettings } from "../settings/store";
import {
  DEFAULT_MEDIA_STUDIO_HEIGHT,
  DEFAULT_MEDIA_STUDIO_WIDTH,
  MIN_MEDIA_STUDIO_HEIGHT,
  MIN_MEDIA_STUDIO_WIDTH,
  normalizeMediaStudioSize,
  useUiPreferences,
} from "../ui/store";
import { ArrowUp, Check, ChevronDown, FolderOpen, Image, Refresh, Search, Sidebar, Trash, X } from "./icons";
import { ComposerSurface } from "./ComposerSurface";
import { GeneratedMedia } from "./GeneratedMedia";
import { InlineMediaControls } from "./InlineMediaControls";
import { ModelPicker } from "./ModelPicker";
import { SheetDialog } from "./SheetDialog";
import { Select } from "./ui";

type MediaModelCatalog = Record<string, Partial<Record<MediaKind, MediaModelInfo[]>>>;

export function MediaManager({
  onClose,
  onManageProviders,
}: {
  onClose: () => void;
  onManageProviders?: () => void;
}) {
  const mediaSettings = useSettings((s) => s.media);
  const setMediaSettings = useSettings((s) => s.setMediaSettings);
  const savedStudioWidth = useUiPreferences((s) => s.mediaStudioWidth);
  const savedStudioHeight = useUiPreferences((s) => s.mediaStudioHeight);
  const setMediaStudioSize = useUiPreferences((s) => s.setMediaStudioSize);
  const [studioSize, setStudioSize] = useState(() => normalizeMediaStudioSize(savedStudioWidth, savedStudioHeight));
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const available = useMemo(() => mediaProviders(providers), [providers]);
  const [providerId, setProviderId] = useState("");
  const selectedProvider = (providerId ? available.find((provider) => provider.id === providerId) : available[0]) ?? null;
  const [kind, setKind] = useState<MediaKind>("image");
  const [model, setModel] = useState("");
  const [prompt, setPrompt] = useState("");
  const [advanced, setAdvanced] = useState(DEFAULT_MEDIA_ADVANCED_INPUT);
  const [modelOptions, setModelOptions] = useState<MediaModelInfo[]>([]);
  const [modelCatalog, setModelCatalog] = useState<MediaModelCatalog>({});
  const [modelPickerOpen, setModelPickerOpen] = useState(false);
  const [modelPickerStyle, setModelPickerStyle] = useState<CSSProperties>();
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const [modelSchema, setModelSchema] = useState<MediaModelSchema | null>(null);
  const [parameterValues, setParameterValues] = useState<Record<string, unknown>>({});
  const [modelsLoading, setModelsLoading] = useState(false);
  const [schemaLoading, setSchemaLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [results, setResults] = useState<MediaGenerationResult[]>([]);
  const [libraryItems, setLibraryItems] = useState<MediaLibraryItem[]>([]);
  const [libraryCursor, setLibraryCursor] = useState<string | null>(null);
  const [libraryQuery, setLibraryQuery] = useState("");
  const [libraryKind, setLibraryKind] = useState<MediaKind | "">("");
  const [libraryProvider, setLibraryProvider] = useState("");
  const [libraryStatus, setLibraryStatus] = useState<MediaLibraryStatus | "">("");
  const [libraryLoading, setLibraryLoading] = useState(false);
  const [libraryOpen, setLibraryOpen] = useState(false);
  const [selectedLibraryId, setSelectedLibraryId] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState("");
  const [privacyMode, setPrivacyModeLabel] = useState("off");
  const pollingKeys = useRef<Set<string>>(new Set());
  const libraryRequest = useRef(0);
  const libraryVisibilityInitialized = useRef(false);
  const reusedModelRef = useRef<string | null>(null);
  const resizeCleanupRef = useRef<(() => void) | null>(null);
  const modelPickerTriggerRef = useRef<HTMLButtonElement>(null);
  const modelPickerPopoverRef = useRef<HTMLDivElement>(null);
  const metadataProvider = Boolean(selectedProvider && supportsMediaMetadataProvider(selectedProvider));
  const mediaPickerRoutes = useMemo(() => {
    const routes = new Map<string, { provider: ProviderInfo; info: MediaModelInfo; kinds: MediaKind[] }>();
    const orderedProviders = selectedProvider
      ? [selectedProvider, ...available.filter((provider) => provider.id !== selectedProvider.id)]
      : available;
    for (const provider of orderedProviders) {
      for (const catalogKind of ["image", "video", "music"] as MediaKind[]) {
        for (const info of modelCatalog[provider.id]?.[catalogKind] ?? []) {
          const existing = routes.get(info.id);
          if (existing?.provider.id === provider.id) {
            if (!existing.kinds.includes(catalogKind)) existing.kinds.push(catalogKind);
          } else if (!existing) {
            routes.set(info.id, { provider, info, kinds: [catalogKind] });
          }
        }
      }
    }
    return routes;
  }, [available, modelCatalog, selectedProvider]);
  const mediaPickerModels = useMemo<ModelInfo[]>(() => Array.from(mediaPickerRoutes)
    .filter(([, route]) => route.kinds.includes(kind))
    .map(([id, route]) => ({
      id,
      display_id: route.info.name ? `${route.info.name} (${id})` : id,
      owned_by: route.provider.name,
      capabilities: {
        imageOutput: route.kinds.includes("image"),
        videoOutput: route.kinds.includes("video"),
        musicOutput: route.kinds.includes("music"),
      },
    })), [kind, mediaPickerRoutes]);
  const favoriteModelIds = useMemo(() => Array.from(new Set(
    Object.values(mediaSettings.favoriteModelIdsByProvider).flat(),
  )), [mediaSettings.favoriteModelIdsByProvider]);
  const selectedModelRoute = mediaPickerRoutes.get(model);
  const selectedModelInfo = selectedModelRoute?.info ?? modelOptions.find((item) => item.id === model);
  const selectedModelLabel = selectedModelInfo?.name
    ? `${selectedModelInfo.name} (${selectedModelInfo.id})`
    : model;
  const selectedLibraryItem = libraryItems.find((item) => item.id === selectedLibraryId)
    ?? (!selectedLibraryId ? libraryItems[0] ?? null : null);
  const latestResult = results[0] ?? null;

  useEffect(() => {
    setStudioSize(normalizeMediaStudioSize(savedStudioWidth, savedStudioHeight));
  }, [savedStudioWidth, savedStudioHeight]);

  useEffect(() => () => resizeCleanupRef.current?.(), []);

  useEffect(() => {
    if (!modelPickerOpen) return;
    const closeOnOutsideClick = (event: MouseEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (modelPickerTriggerRef.current?.contains(target) || modelPickerPopoverRef.current?.contains(target)) return;
      setModelPickerOpen(false);
    };
    const closeOnResize = () => setModelPickerOpen(false);
    document.addEventListener("mousedown", closeOnOutsideClick);
    window.addEventListener("resize", closeOnResize);
    return () => {
      document.removeEventListener("mousedown", closeOnOutsideClick);
      window.removeEventListener("resize", closeOnResize);
    };
  }, [modelPickerOpen]);

  useEffect(() => {
    setConfirmDeleteId("");
  }, [selectedLibraryId]);

  useEffect(() => {
    listProviders().then((next) => {
      const media = mediaProviders(next);
      const saved = useSettings.getState().media;
      const initialProvider = media.find((provider) => provider.id === saved.providerId) ?? media[0];
      const initialModel = initialProvider
        ? saved.modelByProvider[initialProvider.id] || defaultMediaModel(initialProvider)
        : "";
      const key = initialProvider ? mediaPreferenceKey(initialProvider.id, initialModel) : "";
      setProviders(next);
      setProviderId((current) => current || initialProvider?.id || "");
      setModel((current) => current || initialModel);
      setAdvanced((current) => current === DEFAULT_MEDIA_ADVANCED_INPUT && key
        ? saved.advancedByProviderModel[key] ?? defaultMediaAdvanced(initialProvider)
        : current);
      setParameterValues(key ? saved.parametersByProviderModel[key] ?? {} : {});
    });
    getPrivacyMode()
      .then(setPrivacyModeLabel)
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void loadLibrary();
    }, 150);
    return () => window.clearTimeout(timer);
  }, [libraryQuery, libraryKind, libraryProvider, libraryStatus]);

  useEffect(() => {
    if (!libraryItems.some((item) => item.save_state === "running" || item.save_state === "saving")) return;
    const timer = window.setInterval(() => {
      const running = libraryItems.filter((item) => item.save_state === "running");
      if (running.length) {
        void Promise.all(running.map((item) => refreshMediaLibraryItem(item.id)))
          .then(() => loadLibrary())
          .catch(() => loadLibrary());
      } else {
        void loadLibrary();
      }
    }, 3000);
    return () => window.clearInterval(timer);
  }, [libraryItems.map((item) => `${item.id}:${item.save_state}`).join("\u0000")]);

  useEffect(() => {
    if (!selectedProvider) return;
    setProviderId(selectedProvider.id);
    setMediaSettings({ providerId: selectedProvider.id });
    setModel((current) => current || mediaSettings.modelByProvider[selectedProvider.id] || defaultMediaModel(selectedProvider));
  }, [selectedProvider]);

  useEffect(() => {
    if (!available.length) {
      setModelCatalog({});
      return;
    }
    let cancelled = false;
    void Promise.all(available.flatMap((provider) =>
      (["image", "video", "music"] as MediaKind[]).map(async (catalogKind) => {
        try {
          return { providerId: provider.id, kind: catalogKind, models: await listMediaModels(provider.id, catalogKind) };
        } catch {
          return { providerId: provider.id, kind: catalogKind, models: [] as MediaModelInfo[] };
        }
      }),
    )).then((entries) => {
      if (cancelled) return;
      const next: MediaModelCatalog = {};
      for (const entry of entries) {
        next[entry.providerId] = { ...next[entry.providerId], [entry.kind]: entry.models };
      }
      setModelCatalog(next);
    });
    return () => {
      cancelled = true;
    };
  }, [available.map((provider) => provider.id).join("\u0000")]);

  useEffect(() => {
    if (!selectedProvider || !metadataProvider) {
      setModelOptions([]);
      setModelSchema(null);
      setModelsLoading(false);
      setSchemaLoading(false);
      return;
    }
    let cancelled = false;
    setModelsLoading(true);
    setError(null);
    listMediaModels(selectedProvider.id, kind)
      .then((models) => {
        if (cancelled) return;
        setModelOptions(models);
        setModelCatalog((current) => ({
          ...current,
          [selectedProvider.id]: {
            ...current[selectedProvider.id],
            [kind]: models,
          },
        }));
        const savedModel = useSettings.getState().media.modelByProvider[selectedProvider.id];
        const reusedModel = reusedModelRef.current;
        reusedModelRef.current = null;
        const nextModel = reusedModel ?? (savedModel && models.some((item) => item.id === savedModel)
          ? savedModel
          : model && models.some((item) => item.id === model)
            ? model
            : models[0]?.id || defaultMediaModel(selectedProvider));
        if (nextModel && nextModel !== model) {
          applyModel(nextModel, selectedProvider);
        }
      })
      .catch((e) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setModelsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedProvider?.id, metadataProvider, kind]);

  useEffect(() => {
    if (!selectedProvider || !metadataProvider || !model.trim()) {
      setModelSchema(null);
      return;
    }
    let cancelled = false;
    const key = mediaPreferenceKey(selectedProvider.id, model.trim());
    setSchemaLoading(true);
    getMediaModelSchema(selectedProvider.id, model.trim(), kind)
      .then((schema) => {
        if (cancelled) return;
        setModelSchema(schema);
        const saved = useSettings.getState().media.parametersByProviderModel[key];
        setParameterValues({ ...schemaDefaults(schema), ...saved });
      })
      .catch((e) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setSchemaLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedProvider?.id, metadataProvider, kind, model]);

  function applyModel(nextModel: string, provider = selectedProvider) {
    setModel(nextModel);
    if (!provider) return;
    const key = mediaPreferenceKey(provider.id, nextModel);
    const saved = useSettings.getState().media;
    setAdvanced(saved.advancedByProviderModel[key] ?? defaultMediaAdvanced(provider));
    setParameterValues(saved.parametersByProviderModel[key] ?? {});
    setMediaSettings({
      providerId: provider.id,
      modelByProvider: {
        ...saved.modelByProvider,
        [provider.id]: nextModel,
      },
    });
  }

  function applyPickerModel(nextModel: string) {
    const route = mediaPickerRoutes.get(nextModel);
    if (!route) {
      applyModel(nextModel);
      return;
    }
    const nextKind = route.kinds.includes(kind) ? kind : route.kinds[0];
    setProviderId(route.provider.id);
    if (nextKind !== kind) setKind(nextKind);
    applyModel(nextModel, route.provider);
  }

  function toggleMediaModelPicker() {
    if (modelPickerOpen) {
      setModelPickerOpen(false);
      return;
    }
    const rect = modelPickerTriggerRef.current?.getBoundingClientRect();
    if (!rect) return;
    const edge = 12;
    const gap = 6;
    const width = Math.min(480, window.innerWidth - edge * 2);
    const maxHeight = Math.min(440, window.innerHeight - edge * 2);
    const left = Math.max(edge, Math.min(window.innerWidth - width - edge, rect.right - width));
    const below = rect.bottom + gap;
    setModelPickerStyle(below + maxHeight <= window.innerHeight - edge
      ? { left, top: below, width, maxHeight }
      : { left, bottom: window.innerHeight - rect.top + gap, width, maxHeight });
    setModelPickerOpen(true);
  }

  async function loadLibrary(cursor?: string, append = false) {
    const request = ++libraryRequest.current;
    setLibraryLoading(true);
    try {
      const page = await listMediaLibrary({
        query: libraryQuery.trim() || undefined,
        kind: libraryKind || undefined,
        provider: libraryProvider || undefined,
        status: libraryStatus || undefined,
        cursor,
        limit: 40,
      });
      if (request !== libraryRequest.current) return;
      setLibraryItems((current) => append ? [...current, ...page.items] : page.items);
      setLibraryCursor(page.next_cursor ?? null);
      if (!libraryVisibilityInitialized.current) {
        libraryVisibilityInitialized.current = true;
        setLibraryOpen(page.items.length > 0);
      }
      if (!append) {
        setSelectedLibraryId((current) => page.items.some((item) => item.id === current) ? current : page.items[0]?.id ?? "");
      }
    } catch (e) {
      if (request === libraryRequest.current) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (request === libraryRequest.current) setLibraryLoading(false);
    }
  }

  function toggleModelFavorite(modelId: string) {
    const provider = mediaPickerRoutes.get(modelId)?.provider ?? selectedProvider;
    if (!provider || !modelId.trim()) return;
    const saved = useSettings.getState().media;
    const current = saved.favoriteModelIdsByProvider[provider.id] ?? [];
    const next = current.includes(modelId)
      ? current.filter((item) => item !== modelId)
      : [...current, modelId];
    setMediaSettings({
      favoriteModelIdsByProvider: {
        ...saved.favoriteModelIdsByProvider,
        [provider.id]: next,
      },
    });
  }

  function updateAdvanced(value: string) {
    setAdvanced(value);
    if (!selectedProvider || !model.trim()) return;
    const key = mediaPreferenceKey(selectedProvider.id, model.trim());
    const saved = useSettings.getState().media;
    setMediaSettings({
      advancedByProviderModel: {
        ...saved.advancedByProviderModel,
        [key]: value,
      },
    });
  }

  function updateParameter(control: MediaSchemaControl, value: string | boolean) {
    if (!selectedProvider || !model.trim()) return;
    let parsed: unknown;
    try {
      parsed = parseControlValue(control, value);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return;
    }
    const next = { ...parameterValues, [control.key]: parsed };
    setParameterValues(next);
    const key = mediaPreferenceKey(selectedProvider.id, model.trim());
    const saved = useSettings.getState().media;
    setMediaSettings({
      parametersByProviderModel: {
        ...saved.parametersByProviderModel,
        [key]: next,
      },
    });
  }

  async function pollMediaStatus(initial: MediaGenerationResult) {
    const key = `${initial.provider_id}:${initial.id}`;
    if (pollingKeys.current.has(key) || !shouldPollMediaStatus(initial)) return;
    pollingKeys.current.add(key);
    let current = initial;
    try {
      for (let attempt = 0; attempt < mediaPollingMaxAttempts(initial); attempt += 1) {
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
        setResults((items) => items.map((item) => (
          item.provider_id === next.provider_id && item.id === next.id ? { ...item, ...next } : item
        )));
        void loadLibrary();
        if (isTerminalMediaStatus(next.status) || next.media.length > 0) break;
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      pollingKeys.current.delete(key);
    }
  }

  async function submit() {
    const provider = selectedProvider;
    if (!provider) {
      setError("Add an enabled Replicate, fal, or OpenRouter provider first.");
      return;
    }
    if (!model.trim()) {
      setError("Model id is required.");
      return;
    }
    if (metadataProvider && (modelsLoading || !modelOptions.some((item) => item.id === model))) {
      setError("Choose an available media model before generating.");
      return;
    }
    if (!prompt.trim()) {
      setError("Prompt is required.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const result = await generateMedia({
        provider_id: provider.id,
        kind,
        model: model.trim(),
        prompt: prompt.trim(),
        input: inputWithSchemaControls(advanced, metadataProvider ? modelSchema : null, parameterValues),
      });
      setResults((current) => [result, ...current].slice(0, 8));
      if (result.library_id) setSelectedLibraryId(result.library_id);
      void loadLibrary();
      void pollMediaStatus(result);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  function reuseLibraryItem(item: MediaLibraryItem) {
    const provider = available.find((candidate) => candidate.id === item.provider_id);
    reusedModelRef.current = item.model;
    setKind(item.kind);
    setPrompt(item.prompt);
    setAdvanced(JSON.stringify(item.input ?? {}, null, 2));
    setParameterValues({});
    if (provider) {
      setProviderId(provider.id);
      setModel(item.model);
      setMediaSettings({
        providerId: provider.id,
        modelByProvider: {
          ...useSettings.getState().media.modelByProvider,
          [provider.id]: item.model,
        },
      });
    } else {
      setProviderId(item.provider_id);
      setModel(item.model);
      setError(`The original provider ${item.provider} is unavailable. Choose another provider before generating.`);
    }
  }

  async function refreshSelected() {
    if (!selectedLibraryItem) return;
    setLibraryLoading(true);
    setError(null);
    try {
      const next = await refreshMediaLibraryItem(selectedLibraryItem.id);
      setLibraryItems((items) => items.map((item) => item.id === next.id ? next : item));
      window.setTimeout(() => void loadLibrary(), next.save_state === "saving" ? 800 : 0);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLibraryLoading(false);
    }
  }

  async function deleteSelected() {
    if (!selectedLibraryItem) return;
    if (confirmDeleteId !== selectedLibraryItem.id) {
      setConfirmDeleteId(selectedLibraryItem.id);
      return;
    }
    setLibraryLoading(true);
    setError(null);
    try {
      await deleteMediaLibraryItem(selectedLibraryItem.id);
      setConfirmDeleteId("");
      setSelectedLibraryId("");
      await loadLibrary();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLibraryLoading(false);
    }
  }

  async function revealSelected() {
    const path = selectedLibraryItem?.media[0]?.local_path;
    if (!path) return;
    setError(null);
    try {
      await openArtifactLocation(path, "folder");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  function onStudioKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
      event.preventDefault();
      if (!busy) void submit();
    }
  }

  function startStudioResize(event: ReactPointerEvent<HTMLButtonElement>) {
    if (event.button !== 0) return;
    const sheet = event.currentTarget.closest<HTMLElement>(".media-sheet");
    if (!sheet) return;

    event.preventDefault();
    event.stopPropagation();
    resizeCleanupRef.current?.();

    const bounds = sheet.getBoundingClientRect();
    const origin = { x: event.clientX, y: event.clientY, width: bounds.width, height: bounds.height };
    let latest = { width: bounds.width, height: bounds.height };
    let moved = false;

    const cleanup = () => {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
      window.removeEventListener("pointercancel", onPointerCancel);
      document.body.classList.remove("media-studio-resizing");
      resizeCleanupRef.current = null;
    };
    const onPointerMove = (moveEvent: PointerEvent) => {
      moved = true;
      const maxWidth = Math.max(320, window.innerWidth - 24);
      const maxHeight = Math.max(360, window.innerHeight - 24);
      const minWidth = Math.min(MIN_MEDIA_STUDIO_WIDTH, maxWidth);
      const minHeight = Math.min(MIN_MEDIA_STUDIO_HEIGHT, maxHeight);
      latest = {
        width: Math.round(Math.min(Math.max(origin.width + ((moveEvent.clientX - origin.x) * 2), minWidth), maxWidth)),
        height: Math.round(Math.min(Math.max(origin.height + ((moveEvent.clientY - origin.y) * 2), minHeight), maxHeight)),
      };
      setStudioSize(latest);
    };
    const onPointerUp = () => {
      cleanup();
      if (moved) setMediaStudioSize(latest.width, latest.height);
    };
    const onPointerCancel = () => cleanup();

    resizeCleanupRef.current = cleanup;
    document.body.classList.add("media-studio-resizing");
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
    window.addEventListener("pointercancel", onPointerCancel);
  }

  function resizeStudioWithKeyboard(event: KeyboardEvent<HTMLButtonElement>) {
    const step = event.shiftKey ? 64 : 32;
    const sheet = event.currentTarget.closest<HTMLElement>(".media-sheet");
    const bounds = sheet?.getBoundingClientRect();
    const current = bounds
      ? { width: bounds.width, height: bounds.height }
      : studioSize;
    let next: { width: number; height: number } | null = null;

    if (event.key === "ArrowLeft") next = { ...current, width: current.width - step };
    if (event.key === "ArrowRight") next = { ...current, width: current.width + step };
    if (event.key === "ArrowUp") next = { ...current, height: current.height - step };
    if (event.key === "ArrowDown") next = { ...current, height: current.height + step };
    if (event.key === "Home") next = { width: DEFAULT_MEDIA_STUDIO_WIDTH, height: DEFAULT_MEDIA_STUDIO_HEIGHT };
    if (!next) return;

    event.preventDefault();
    event.stopPropagation();
    const normalized = normalizeMediaStudioSize(next.width, next.height);
    setStudioSize(normalized);
    setMediaStudioSize(normalized.width, normalized.height);
  }

  function resetStudioSize() {
    const size = { width: DEFAULT_MEDIA_STUDIO_WIDTH, height: DEFAULT_MEDIA_STUDIO_HEIGHT };
    setStudioSize(size);
    setMediaStudioSize(size.width, size.height);
  }

  const stageMedia = selectedLibraryItem?.media[0] ?? latestResult?.media[0];
  const stageModel = selectedLibraryItem?.model ?? latestResult?.model ?? model;
  const stageKind = selectedLibraryItem?.kind ?? latestResult?.kind ?? kind;
  const stageStatus = selectedLibraryItem?.save_state ?? latestResult?.save_state ?? (busy ? "running" : null);
  const stageEmptyTitle = stageStatus === "running"
    ? `Generating ${stageKind}...`
    : stageStatus === "saving"
      ? "Saving locally..."
      : stageStatus === "failed"
        ? "This run failed"
        : "Your next output will appear here";
  const stageEmptyDetail = stageStatus === "running"
    ? "The output will appear here when it is ready."
    : stageStatus === "saving"
      ? "Milim is adding the finished output to your local library."
      : stageStatus === "failed"
        ? selectedLibraryItem ? "Refresh it to retry, or reuse its settings." : "Review the error details and try again."
        : "Choose a model, write a prompt, and generate.";
  const showLibraryFilters = libraryItems.length > 0 || Boolean(
    libraryQuery.trim() || libraryKind || libraryProvider || libraryStatus,
  );
  const mediaSettingsLabel = selectedProvider
    ? `${selectedProvider.name} · ${model || `Choose a ${kind} model`}`
    : "Choose a provider";
  const selectedProviderAvailable = Boolean(selectedProvider);
  const selectedModelAvailable = metadataProvider
    ? !modelsLoading && modelOptions.some((item) => item.id === model)
    : Boolean(model.trim());
  const mediaSheetStyle = {
    width: studioSize.width,
    height: studioSize.height,
  } satisfies CSSProperties;

  return (
    <SheetDialog
      title="Media studio"
      className="sheet media-sheet"
      testId="media-generator"
      style={mediaSheetStyle}
      onClose={onClose}
    >
      <div className="media-studio" onKeyDown={onStudioKeyDown}>
        <div className="sheet-header media-studio-header">
          <div>
            <h2>Media studio</h2>
            <p>Quick generations here. Iteration stays in chat.</p>
          </div>
          <button className="icon-btn" type="button" onClick={onClose} title="Close" aria-label="Close media studio">
            <X size={15} />
          </button>
        </div>

        <div className="media-grid">
          <section className="media-form media-composer-dock" aria-label="Media generator">
            <ComposerSurface className="media-composer-surface">
              <div className="control-bar media-control-bar">
                <div className="chips">
                  <div className="chip-wrap media-model-picker-wrap">
                    <button
                      ref={modelPickerTriggerRef}
                      className="chip chip-model media-model-picker-trigger"
                      data-testid="media-model-picker-trigger"
                      type="button"
                      title={mediaSettingsLabel}
                      aria-label={`Choose ${kind} model${selectedModelLabel ? `, current model ${selectedModelLabel}` : ""}`}
                      aria-haspopup="dialog"
                      aria-expanded={modelPickerOpen}
                      disabled={!selectedProvider && !onManageProviders}
                      onClick={selectedProvider ? toggleMediaModelPicker : onManageProviders}
                    >
                      <span className={`dot ${selectedModelAvailable ? "dot-green" : "dot-yellow"}`} />
                      <span className="chip-label">{modelsLoading ? `Loading ${kind} models...` : selectedModelLabel || `Choose a ${kind} model`}</span>
                      <ChevronDown size={12} className="chip-chev" />
                    </button>
                  </div>
                  <div className="control-inline-slot">
                    <InlineMediaControls
                      providerName={selectedProvider?.name || "Media"}
                      model={model}
                      kind={kind}
                      supportedKinds={["image", "video", "music"]}
                      schema={modelSchema}
                      schemaLoading={schemaLoading}
                      parameterValues={parameterValues}
                      advanced={advanced}
                      error={error}
                      onKindChange={(nextKind) => {
                        setKind(nextKind);
                        setModel("");
                        setModelSchema(null);
                      }}
                      onParameterChange={updateParameter}
                      onAdvancedChange={updateAdvanced}
                    />
                  </div>
                </div>
              </div>

              <div className="composer comfortable media-composer-box">
                <div className="composer-input-wrap">
                  <textarea
                    className="composer-input media-composer-prompt"
                    data-testid="media-prompt-input"
                    value={prompt}
                    rows={3}
                    aria-label="Media prompt"
                    placeholder={kind === "music" ? "Warm instrumental synthwave with a steady pulse..." : "Product photo on a clean workbench, natural side light..."}
                    onChange={(e) => setPrompt(e.target.value)}
                  />
                </div>
                <div className="composer-bar media-composer-bar">
                  <div className="composer-tools" />
                  <div className="composer-send media-composer-send">
                  <div className="media-privacy" data-testid="media-privacy">
                    Privacy <strong>{privacyMode}</strong>
                    <span>{privacyMode === "block" ? "PII blocks request" : privacyMode === "redact" ? "PII removed before upload" : "Prompt sent unchanged"}</span>
                  </div>
                  <kbd className="media-generate-shortcut">Ctrl/Cmd + Enter</kbd>
                  <button
                    className="send-btn media-composer-send-btn"
                    data-testid="media-generate"
                    type="button"
                    title={`${busy ? "Generating" : `Generate ${kind}`} (Ctrl/Cmd + Enter)`}
                    aria-label={busy ? `Generating ${kind}` : `Generate ${kind}`}
                    disabled={busy || !selectedProviderAvailable || !selectedModelAvailable}
                    onClick={() => void submit()}
                  >
                    <ArrowUp size={17} />
                  </button>
                  </div>
                </div>
              </div>
            </ComposerSurface>
          </section>

          <section className={`media-workspace${libraryOpen ? " library-open" : ""}`} aria-label="Generated media and library">
            <div className="media-stage" data-testid="media-stage">
              <div className="media-stage-head">
                <div className="media-stage-title">
                  <strong>Output</strong>
                  <span title={stageModel}>{stageModel || "Nothing selected"}</span>
                </div>
                <div className="media-stage-head-actions">
                  {stageStatus && stageStatus !== "ready" && <span className={`media-status ${stageStatus}`} role="status" aria-live="polite">{stageStatus}</span>}
                  <button
                    className={`btn-ghost media-library-toggle${libraryOpen ? " active" : ""}`}
                    type="button"
                    aria-label={`${libraryOpen ? "Close" : "Open"} local library, ${libraryItems.length} ${libraryItems.length === 1 ? "item" : "items"}`}
                    aria-controls="media-library-sidebar"
                    aria-expanded={libraryOpen}
                    onClick={() => {
                      libraryVisibilityInitialized.current = true;
                      setLibraryOpen((open) => !open);
                    }}
                  >
                    <Sidebar size={14} />
                    <span>Library</span>
                    <span className="media-library-count" aria-hidden="true">{libraryItems.length}</span>
                  </button>
                </div>
              </div>
              <div className={`media-stage-preview${stageMedia ? " has-media" : ""}`}>
                {stageMedia ? (
                  <GeneratedMedia
                    item={stageMedia}
                    alt={`Generated ${stageKind} from ${stageModel}`}
                    onOpenExternal={(url) => void openExternalUrl(url)}
                  />
                ) : (
                  <div className="media-empty">
                    <Image size={28} />
                    <strong>{stageEmptyTitle}</strong>
                    <span>{stageEmptyDetail}</span>
                  </div>
                )}
              </div>
              {selectedLibraryItem && (
                <div className="media-stage-meta">
                  <p>{selectedLibraryItem.prompt}</p>
                  <div className="media-stage-actions">
                    <button className="btn-ghost" type="button" onClick={() => reuseLibraryItem(selectedLibraryItem)}>Use settings</button>
                    {(selectedLibraryItem.save_state === "running" || selectedLibraryItem.save_state === "failed") && (
                      <button className="btn-ghost" type="button" disabled={libraryLoading} onClick={() => void refreshSelected()}>
                        <Refresh size={13} /> Refresh
                      </button>
                    )}
                    {selectedLibraryItem.media[0]?.local_path && (
                      <button className="btn-ghost" type="button" onClick={() => void revealSelected()}>
                        <FolderOpen size={13} /> Reveal
                      </button>
                    )}
                    <button className="btn-ghost danger" type="button" disabled={libraryLoading} onClick={() => void deleteSelected()}>
                      {confirmDeleteId === selectedLibraryItem.id ? <Check size={13} /> : <Trash size={13} />}
                      {confirmDeleteId === selectedLibraryItem.id ? "Confirm delete" : "Delete"}
                    </button>
                  </div>
                  {selectedLibraryItem.error && <div className="artifact-error" role="alert">{selectedLibraryItem.error}</div>}
                </div>
              )}
            </div>

            {libraryOpen && <aside className="media-library" id="media-library-sidebar" aria-label="Local library">
              <div className="media-library-head">
                <div>
                  <span className="media-eyebrow">Local library</span>
                  <strong>{libraryItems.length ? `${libraryItems.length} shown` : "Generated media"}</strong>
                </div>
                <div className="media-library-search">
                  <Search size={13} aria-hidden="true" />
                  <input value={libraryQuery} aria-label="Search media library" placeholder="Search prompts or models..." onChange={(e) => setLibraryQuery(e.target.value)} />
                </div>
              </div>
              {showLibraryFilters && (
                <div className="media-library-filters">
                  <div className="media-filter-tabs" aria-label="Filter library by media type">
                    {(["", "image", "video", "music"] as const).map((value) => (
                      <button key={value || "all"} type="button" className={libraryKind === value ? "active" : ""} aria-pressed={libraryKind === value} onClick={() => setLibraryKind(value)}>
                        {value || "All"}
                      </button>
                    ))}
                  </div>
                  <Select
                    value={libraryProvider}
                    placeholder="All providers"
                    options={[{ label: "All providers", value: "" }, ...available.map((provider) => ({ label: provider.name, value: provider.id }))]}
                    onChange={setLibraryProvider}
                  />
                  <Select
                    value={libraryStatus}
                    placeholder="Any status"
                    options={[
                      { label: "Any status", value: "" },
                      { label: "Ready", value: "ready" },
                      { label: "Running", value: "running" },
                      { label: "Saving", value: "saving" },
                      { label: "Failed", value: "failed" },
                    ]}
                    onChange={(value) => setLibraryStatus(value as MediaLibraryStatus | "")}
                  />
                </div>
              )}

              <div className="media-library-grid" aria-busy={libraryLoading}>
                {libraryItems.map((item) => (
                  <article
                    className={`media-library-card${item.id === selectedLibraryItem?.id ? " active" : ""}`}
                    data-testid="media-library-item"
                    aria-current={item.id === selectedLibraryItem?.id ? "true" : undefined}
                    key={item.id}
                  >
                    <div className="media-library-thumb">
                      {item.media[0] && item.kind !== "music" ? (
                        <GeneratedMedia item={item.media[0]} alt={`Select generated ${item.kind} from ${item.model}`} onActivate={() => setSelectedLibraryId(item.id)} />
                      ) : (
                        <button type="button" onClick={() => setSelectedLibraryId(item.id)} aria-label={`Select ${item.kind} from ${item.model}`}>
                          <Image size={20} />
                        </button>
                      )}
                      {item.save_state !== "ready" && <span className={`media-status ${item.save_state}`}>{item.save_state}</span>}
                    </div>
                    <button className="media-library-card-body" type="button" onClick={() => setSelectedLibraryId(item.id)}>
                      <strong title={item.prompt}>{item.prompt}</strong>
                      <span className="media-library-card-meta" aria-label={`${item.provider}, ${item.model}`}>
                        <span className="media-library-card-provider">{item.provider}</span>
                        <span className="media-library-card-model" title={item.model}>{item.model.split("/").pop() || item.model}</span>
                      </span>
                    </button>
                  </article>
                ))}
                {!libraryItems.length && libraryLoading && (
                  <div className="media-library-empty" role="status">Loading local library...</div>
                )}
                {!libraryItems.length && !libraryLoading && (
                  <div className="media-library-empty">
                    <Image size={20} />
                    <span>{libraryQuery || libraryKind || libraryProvider || libraryStatus ? "No media matches these filters." : "Completed chat and studio generations will be saved here."}</span>
                  </div>
                )}
              </div>
              {libraryCursor && (
                <button className="btn-ghost media-load-more" type="button" disabled={libraryLoading} onClick={() => void loadLibrary(libraryCursor, true)}>
                  {libraryLoading ? "Loading..." : "Load more"}
                </button>
              )}
            </aside>}
          </section>
        </div>
        {modelPickerOpen && modelPickerStyle && createPortal(
          <div
            ref={modelPickerPopoverRef}
            className="media-model-picker-popover"
            data-native-preview-blocker="true"
            style={modelPickerStyle}
          >
            <ModelPicker
              models={mediaPickerModels}
              model={model}
              onModel={(selection) => applyPickerModel(selection.model)}
              onClose={() => setModelPickerOpen(false)}
              showManagementActions={false}
              favoriteIds={favoriteModelIds}
              favoritesOnlyValue={favoritesOnly}
              onToggleFavorite={toggleModelFavorite}
              onFavoritesOnlyChange={setFavoritesOnly}
              searchPlaceholder={`Search ${kind === "music" ? "audio" : kind} models...`}
              emptyMessage={`No ${kind === "music" ? "audio" : kind} models available.`}
            />
          </div>,
          document.body,
        )}
        <button
          className="media-sheet-resize-handle"
          data-testid="media-studio-resize-handle"
          type="button"
          aria-label="Resize media studio"
          title="Drag to resize. Use arrow keys for precise sizing; Home or double-click resets."
          onPointerDown={startStudioResize}
          onKeyDown={resizeStudioWithKeyboard}
          onDoubleClick={resetStudioSize}
        >
        </button>
      </div>
    </SheetDialog>
  );
}

export default MediaManager;
