import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
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
  DEFAULT_MEDIA_COMPOSER_WIDTH,
  DEFAULT_MEDIA_LIBRARY_WIDTH,
  DEFAULT_MEDIA_STUDIO_HEIGHT,
  DEFAULT_MEDIA_STUDIO_WIDTH,
  MAX_SIDEBAR_WIDTH,
  MIN_MEDIA_STUDIO_HEIGHT,
  MIN_MEDIA_STUDIO_WIDTH,
  MIN_SIDEBAR_WIDTH,
  normalizeMediaStudioSize,
  normalizeSidebarWidth,
  useUiPreferences,
} from "../ui/store";
import { ArrowUp, Check, ChevronDown, FolderOpen, Image, Refresh, Search, Sidebar, Sparkles, Trash, Volume2, X } from "./icons";
import { ComposerSurface } from "./ComposerSurface";
import { GeneratedMedia } from "./GeneratedMedia";
import { InlineMediaControls } from "./InlineMediaControls";
import { ModelPicker } from "./ModelPicker";
import { PaneResizeHandle } from "./PaneResizeHandle";
import { ProvidersManager } from "./ProvidersManager";
import { SheetDialog } from "./SheetDialog";
import { Select } from "./ui";

type MediaModelCatalog = Record<string, Partial<Record<MediaKind, MediaModelInfo[]>>>;
type GenerationPhase = "idle" | "submitting" | "failed";
type LibraryAction = "refresh" | "delete" | "reveal";
type LibraryFeedback = { id: string; label: string; message: string };
type StageRequest = { kind: MediaKind; model: string; prompt: string };
type MediaSidePanel = "composer" | "library";

const MEDIA_PANEL_KEYBOARD_STEP = 32;
const MEDIA_PANEL_COLLAPSE_OVERSHOOT = 96;
const MEDIA_PANEL_SNAP_ANIMATION_MS = 180;

function schemaDraftFromInput(schema: MediaModelSchema, input: Record<string, unknown>) {
  const advanced = structuredClone(input);
  const values: Record<string, unknown> = {};
  for (const control of schema.controls) {
    if (!control.path.length) continue;
    let source: unknown = input;
    let target: unknown = advanced;
    for (const segment of control.path.slice(0, -1)) {
      source = source && typeof source === "object" && !Array.isArray(source)
        ? (source as Record<string, unknown>)[segment]
        : undefined;
      target = target && typeof target === "object" && !Array.isArray(target)
        ? (target as Record<string, unknown>)[segment]
        : undefined;
    }
    const key = control.path[control.path.length - 1];
    if (source && typeof source === "object" && !Array.isArray(source) && Object.prototype.hasOwnProperty.call(source, key)) {
      values[control.key] = (source as Record<string, unknown>)[key];
    }
    if (target && typeof target === "object" && !Array.isArray(target)) {
      delete (target as Record<string, unknown>)[key];
    }
  }
  return { advanced, values };
}

export function MediaManager({ onClose }: { onClose: () => void }) {
  const mediaSettings = useSettings((s) => s.media);
  const setMediaSettings = useSettings((s) => s.setMediaSettings);
  const savedStudioWidth = useUiPreferences((s) => s.mediaStudioWidth);
  const savedStudioHeight = useUiPreferences((s) => s.mediaStudioHeight);
  const composerPlacement = useUiPreferences((s) => s.mediaComposerPlacement);
  const composerWidth = useUiPreferences((s) => s.mediaComposerWidth);
  const libraryWidth = useUiPreferences((s) => s.mediaLibraryWidth);
  const setMediaStudioSize = useUiPreferences((s) => s.setMediaStudioSize);
  const setComposerPlacement = useUiPreferences((s) => s.setMediaComposerPlacement);
  const setComposerWidth = useUiPreferences((s) => s.setMediaComposerWidth);
  const setLibraryWidth = useUiPreferences((s) => s.setMediaLibraryWidth);
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
  const [mediaSettingsOpen, setMediaSettingsOpen] = useState(false);
  const [modelPickerStyle, setModelPickerStyle] = useState<CSSProperties>();
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const [modelSchema, setModelSchema] = useState<MediaModelSchema | null>(null);
  const [schemaVersion, setSchemaVersion] = useState(0);
  const [parameterValues, setParameterValues] = useState<Record<string, unknown>>({});
  const [modelsLoading, setModelsLoading] = useState(false);
  const [schemaLoading, setSchemaLoading] = useState(false);
  const [generationPhase, setGenerationPhase] = useState<GenerationPhase>("idle");
  const [generationError, setGenerationError] = useState<string | null>(null);
  const [stageRequest, setStageRequest] = useState<StageRequest | null>(null);
  const [results, setResults] = useState<MediaGenerationResult[]>([]);
  const [libraryItems, setLibraryItems] = useState<MediaLibraryItem[]>([]);
  const [libraryCursor, setLibraryCursor] = useState<string | null>(null);
  const [libraryQuery, setLibraryQuery] = useState("");
  const [libraryKind, setLibraryKind] = useState<MediaKind | "">("");
  const [libraryProvider, setLibraryProvider] = useState("");
  const [libraryStatus, setLibraryStatus] = useState<MediaLibraryStatus | "">("");
  const [libraryLoading, setLibraryLoading] = useState(false);
  const [libraryOpen, setLibraryOpen] = useState(() => (
    typeof window !== "undefined" && Math.min(savedStudioWidth, window.innerWidth - 24) >= 960
  ));
  const [selectedLibraryId, setSelectedLibraryId] = useState("");
  const [libraryListError, setLibraryListError] = useState<string | null>(null);
  const [libraryAction, setLibraryAction] = useState<{ id: string; action: LibraryAction } | null>(null);
  const [libraryActionError, setLibraryActionError] = useState<LibraryFeedback | null>(null);
  const [libraryNotice, setLibraryNotice] = useState<LibraryFeedback | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState("");
  const [providersOpen, setProvidersOpen] = useState(false);
  const [providersVersion, setProvidersVersion] = useState(0);
  const [stageVariantIndex, setStageVariantIndex] = useState(0);
  const [privacyMode, setPrivacyModeLabel] = useState("off");
  const [resizingPanel, setResizingPanel] = useState<MediaSidePanel | null>(null);
  const pollingKeys = useRef<Set<string>>(new Set());
  const generationInFlightRef = useRef(false);
  const generationRunRef = useRef(0);
  const libraryRequest = useRef(0);
  const libraryLoadingRef = useRef(false);
  const libraryLoadedRef = useRef(false);
  const providerRequest = useRef(0);
  const reusedModelRef = useRef<string | null>(null);
  const reusedInputRef = useRef<{
    providerId: string;
    model: string;
    kind: MediaKind;
    input: Record<string, unknown>;
  } | null>(null);
  const preserveProviderDraftRef = useRef(false);
  const resizeCleanupRef = useRef<(() => void) | null>(null);
  const deleteConfirmTimerRef = useRef<number | null>(null);
  const promptRef = useRef<HTMLTextAreaElement>(null);
  const modelPickerTriggerRef = useRef<HTMLButtonElement>(null);
  const modelPickerPopoverRef = useRef<HTMLDivElement>(null);
  const libraryToggleRef = useRef<HTMLButtonElement>(null);
  const mediaGridRef = useRef<HTMLDivElement>(null);
  const variantRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const libraryCardRefs = useRef<Map<string, HTMLButtonElement>>(new Map());
  const selectedLibraryIdRef = useRef("");
  const libraryFiltersRef = useRef({
    query: "",
    kind: "" as MediaKind | "",
    provider: "",
    status: "" as MediaLibraryStatus | "",
  });
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
  const selectedLibraryItem = libraryItems.find((item) => item.id === selectedLibraryId) ?? null;
  const latestResult = results[0] ?? null;
  const busy = generationPhase === "submitting";
  selectedLibraryIdRef.current = selectedLibraryId;
  libraryLoadingRef.current = libraryLoading;
  libraryFiltersRef.current = {
    query: libraryQuery,
    kind: libraryKind,
    provider: libraryProvider,
    status: libraryStatus,
  };

  useEffect(() => {
    setStudioSize(normalizeMediaStudioSize(savedStudioWidth, savedStudioHeight));
  }, [savedStudioWidth, savedStudioHeight]);

  useEffect(() => () => {
    resizeCleanupRef.current?.();
    generationRunRef.current += 1;
    if (deleteConfirmTimerRef.current !== null) window.clearTimeout(deleteConfirmTimerRef.current);
  }, []);

  useEffect(() => {
    const textarea = promptRef.current;
    if (!textarea) return;
    if (composerPlacement === "side") {
      textarea.style.height = "";
      textarea.style.overflowY = "auto";
      return;
    }
    textarea.style.height = "0px";
    textarea.style.height = `${textarea.scrollHeight}px`;
    textarea.style.overflowY = "auto";
  }, [composerPlacement, prompt, providersOpen]);

  useEffect(() => {
    if (!modelPickerOpen) return;
    const closeOnOutsideClick = (event: MouseEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (modelPickerTriggerRef.current?.contains(target) || modelPickerPopoverRef.current?.contains(target)) return;
      closeModelPicker(false);
    };
    const closeOnResize = () => closeModelPicker(false);
    document.addEventListener("mousedown", closeOnOutsideClick);
    window.addEventListener("resize", closeOnResize);
    return () => {
      document.removeEventListener("mousedown", closeOnOutsideClick);
      window.removeEventListener("resize", closeOnResize);
    };
  }, [modelPickerOpen]);

  useEffect(() => {
    if (deleteConfirmTimerRef.current !== null) window.clearTimeout(deleteConfirmTimerRef.current);
    deleteConfirmTimerRef.current = null;
    setConfirmDeleteId("");
    setLibraryActionError(null);
  }, [selectedLibraryId, libraryQuery, libraryKind, libraryProvider, libraryStatus, libraryOpen]);

  useEffect(() => {
    void refreshProviders();
    getPrivacyMode()
      .then(setPrivacyModeLabel)
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    const quiet = !libraryLoadedRef.current;
    const timer = window.setTimeout(() => {
      libraryLoadedRef.current = true;
      void loadLibrary(undefined, false, quiet);
    }, 150);
    return () => window.clearTimeout(timer);
  }, [libraryQuery, libraryKind, libraryProvider, libraryStatus]);

  useEffect(() => {
    if (!libraryItems.some((item) => item.save_state === "running" || item.save_state === "saving")) return;
    const timer = window.setInterval(() => {
      const running = libraryItems.filter((item) => item.save_state === "running");
      if (running.length) {
        void Promise.all(running.map((item) => refreshMediaLibraryItem(item.id)))
          .then(() => loadLibrary(undefined, false, true))
          .catch(() => loadLibrary(undefined, false, true));
      } else {
        void loadLibrary(undefined, false, true);
      }
    }, 3000);
    return () => window.clearInterval(timer);
  }, [
    libraryItems.map((item) => `${item.id}:${item.save_state}`).join("\u0000"),
    libraryQuery,
    libraryKind,
    libraryProvider,
    libraryStatus,
  ]);

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
  }, [available.map((provider) => provider.id).join("\u0000"), providersVersion]);

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
    setModelOptions([]);
    setModelSchema(null);
    setSchemaLoading(true);
    setGenerationError(null);
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
        if (!nextModel || !models.some((item) => item.id === nextModel)) {
          preserveProviderDraftRef.current = false;
        }
      })
      .catch((e) => {
        if (!cancelled) {
          preserveProviderDraftRef.current = false;
          setGenerationError(e instanceof Error ? e.message : String(e));
        }
      })
      .finally(() => {
        if (!cancelled) setModelsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedProvider?.id, metadataProvider, kind, providersVersion]);

  useEffect(() => {
    const selectedModelReady = modelOptions.some((item) => item.id === model);
    if (!selectedProvider || !metadataProvider || !model.trim() || modelsLoading || !selectedModelReady) {
      setModelSchema(null);
      setSchemaLoading(Boolean(selectedProvider && metadataProvider && model.trim() && modelsLoading));
      return;
    }
    let cancelled = false;
    const key = mediaPreferenceKey(selectedProvider.id, model.trim());
    setSchemaLoading(true);
    setGenerationError(null);
    getMediaModelSchema(selectedProvider.id, model.trim(), kind)
      .then((schema) => {
        if (cancelled) return;
        setModelSchema(schema);
        const reused = reusedInputRef.current;
        if (
          reused &&
          reused.providerId === selectedProvider.id &&
          reused.model === model &&
          reused.kind === kind
        ) {
          reusedInputRef.current = null;
          applyReusedInput(schema, reused.input);
        } else if (preserveProviderDraftRef.current) {
          preserveProviderDraftRef.current = false;
        } else {
          const saved = useSettings.getState().media.parametersByProviderModel[key];
          setParameterValues({ ...schemaDefaults(schema), ...saved });
        }
      })
      .catch((e) => {
        if (!cancelled) {
          preserveProviderDraftRef.current = false;
          setGenerationError(e instanceof Error ? e.message : String(e));
        }
      })
      .finally(() => {
        if (!cancelled) setSchemaLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [
    selectedProvider?.id,
    metadataProvider,
    kind,
    model,
    modelsLoading,
    modelOptions.map((item) => item.id).join("\u0000"),
    schemaVersion,
  ]);

  function resetGenerationFeedback() {
    if (generationInFlightRef.current) return;
    setGenerationPhase("idle");
    setGenerationError(null);
  }

  function applyReusedInput(schema: MediaModelSchema, input: Record<string, unknown>) {
    const draft = schemaDraftFromInput(schema, input);
    setAdvanced(JSON.stringify(draft.advanced, null, 2));
    setParameterValues({ ...schemaDefaults(schema), ...draft.values });
  }

  function initializeProvider(provider: ProviderInfo, preserveDraft = false) {
    const saved = useSettings.getState().media;
    const nextModel = saved.modelByProvider[provider.id] || defaultMediaModel(provider);
    const key = mediaPreferenceKey(provider.id, nextModel);
    preserveProviderDraftRef.current = preserveDraft && supportsMediaMetadataProvider(provider);
    setProviderId(provider.id);
    setModel(nextModel);
    setModelOptions([]);
    setModelSchema(null);
    setModelsLoading(supportsMediaMetadataProvider(provider));
    setSchemaLoading(Boolean(nextModel.trim() && supportsMediaMetadataProvider(provider)));
    setGenerationPhase("idle");
    setGenerationError(null);
    if (!preserveDraft) {
      setAdvanced(saved.advancedByProviderModel[key] ?? defaultMediaAdvanced(provider));
      setParameterValues(saved.parametersByProviderModel[key] ?? {});
    }
    setMediaSettings({
      providerId: provider.id,
      modelByProvider: {
        ...saved.modelByProvider,
        [provider.id]: nextModel,
      },
    });
  }

  async function refreshProviders(preserveDraft = false) {
    const request = ++providerRequest.current;
    try {
      const next = await listProviders();
      if (request !== providerRequest.current) return;
      const media = mediaProviders(next);
      const currentProvider = media.find((provider) => provider.id === providerId);
      const savedProvider = media.find((provider) => provider.id === useSettings.getState().media.providerId);
      const nextProvider = currentProvider ?? savedProvider ?? media[0] ?? null;
      if (preserveDraft && nextProvider?.id === providerId) {
        preserveProviderDraftRef.current = supportsMediaMetadataProvider(nextProvider);
      }
      setProviders(next);
      setProvidersVersion((version) => version + 1);
      if (nextProvider?.id !== providerId) {
        if (nextProvider) initializeProvider(nextProvider, preserveDraft);
        else {
          preserveProviderDraftRef.current = false;
          setProviderId("");
          setModel("");
          setModelOptions([]);
          setModelSchema(null);
          setModelsLoading(false);
          setSchemaLoading(false);
        }
      }
    } catch (e) {
      if (request === providerRequest.current) {
        setGenerationError(e instanceof Error ? e.message : String(e));
      }
    }
  }

  function applyModel(nextModel: string, provider = selectedProvider) {
    const providerChanged = provider?.id !== selectedProvider?.id;
    const identityChanged = providerChanged || nextModel !== model;
    setModel(nextModel);
    if (identityChanged) {
      setModelSchema(null);
      setSchemaLoading(Boolean(provider && nextModel.trim() && supportsMediaMetadataProvider(provider)));
      if (providerChanged) {
        setModelOptions([]);
        setModelsLoading(Boolean(provider && supportsMediaMetadataProvider(provider)));
      }
    } else if (!modelSchema && provider && nextModel.trim() && supportsMediaMetadataProvider(provider)) {
      setSchemaLoading(true);
      setSchemaVersion((version) => version + 1);
    }
    resetGenerationFeedback();
    if (!provider) return;
    const key = mediaPreferenceKey(provider.id, nextModel);
    const saved = useSettings.getState().media;
    const reused = reusedInputRef.current;
    const preserveValues = preserveProviderDraftRef.current || Boolean(
      reused && reused.providerId === provider.id && reused.model === nextModel,
    );
    if (!preserveValues) {
      setAdvanced(saved.advancedByProviderModel[key] ?? defaultMediaAdvanced(provider));
      setParameterValues(saved.parametersByProviderModel[key] ?? {});
    }
    setMediaSettings({
      providerId: provider.id,
      modelByProvider: {
        ...saved.modelByProvider,
        [provider.id]: nextModel,
      },
    });
  }

  function applyPickerModel(nextModel: string) {
    preserveProviderDraftRef.current = false;
    reusedInputRef.current = null;
    const route = mediaPickerRoutes.get(nextModel);
    if (!route) {
      applyModel(nextModel);
      return;
    }
    const nextKind = route.kinds.includes(kind) ? kind : route.kinds[0];
    setProviderId(route.provider.id);
    if (nextKind !== kind) {
      setKind(nextKind);
      setModelOptions([]);
      setModelSchema(null);
      setModelsLoading(true);
      setSchemaLoading(true);
    }
    applyModel(nextModel, route.provider);
  }

  function closeModelPicker(restoreFocus = true) {
    setModelPickerOpen(false);
    if (restoreFocus) window.requestAnimationFrame(() => modelPickerTriggerRef.current?.focus());
  }

  function toggleMediaModelPicker() {
    if (modelPickerOpen) {
      closeModelPicker(false);
      return;
    }
    const trigger = modelPickerTriggerRef.current;
    const sheet = trigger?.closest<HTMLElement>(".media-sheet");
    const rect = trigger?.getBoundingClientRect();
    const bounds = sheet?.getBoundingClientRect();
    if (!rect || !bounds) return;
    setMediaSettingsOpen(false);
    const edge = 12;
    const gap = 6;
    const width = Math.min(480, bounds.width - edge * 2);
    const left = Math.max(edge, Math.min(bounds.width - width - edge, rect.right - bounds.left - width));
    const below = Math.max(0, bounds.bottom - edge - rect.bottom - gap);
    const above = Math.max(0, rect.top - bounds.top - gap - edge);
    const openBelow = below >= above;
    const maxHeight = Math.min(440, openBelow ? below : above);
    setModelPickerStyle(openBelow
      ? { left, top: rect.bottom - bounds.top + gap, width, maxHeight }
      : { left, bottom: bounds.bottom - rect.top + gap, width, maxHeight });
    setModelPickerOpen(true);
  }

  async function loadLibrary(cursor?: string, append = false, quiet = false) {
    if (quiet && libraryLoadingRef.current) return;
    const request = ++libraryRequest.current;
    if (!quiet) {
      libraryLoadingRef.current = true;
      setLibraryLoading(true);
      setLibraryListError(null);
    }
    try {
      const filters = libraryFiltersRef.current;
      const page = await listMediaLibrary({
        query: filters.query.trim() || undefined,
        kind: filters.kind || undefined,
        provider: filters.provider || undefined,
        status: filters.status || undefined,
        cursor,
        limit: 40,
      });
      if (request !== libraryRequest.current) return;
      setLibraryItems((current) => append ? [...current, ...page.items] : page.items);
      setLibraryCursor(page.next_cursor ?? null);
      if (!append) {
        setSelectedLibraryId((current) => page.items.some((item) => item.id === current) ? current : "");
      }
    } catch (e) {
      if (!quiet && request === libraryRequest.current) {
        setLibraryListError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      if (request === libraryRequest.current) {
        libraryLoadingRef.current = false;
        setLibraryLoading(false);
      }
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
    resetGenerationFeedback();
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
      parsed = typeof value === "string" && (control.kind === "array" || control.kind === "json")
        ? value
        : parseControlValue(control, value);
    } catch (e) {
      setGenerationError(e instanceof Error ? e.message : String(e));
      return;
    }
    const next = { ...parameterValues, [control.key]: parsed };
    setParameterValues(next);
    resetGenerationFeedback();
    const key = mediaPreferenceKey(selectedProvider.id, model.trim());
    const saved = useSettings.getState().media;
    setMediaSettings({
      parametersByProviderModel: {
        ...saved.parametersByProviderModel,
        [key]: next,
      },
    });
  }

  async function pollMediaStatus(initial: MediaGenerationResult, run: number) {
    const key = `${initial.provider_id}:${initial.id}`;
    if (pollingKeys.current.has(key) || !shouldPollMediaStatus(initial)) return;
    pollingKeys.current.add(key);
    let current = initial;
    try {
      for (let attempt = 0; attempt < mediaPollingMaxAttempts(initial); attempt += 1) {
        if (run !== generationRunRef.current) break;
        await new Promise((resolve) => window.setTimeout(resolve, 3000));
        if (run !== generationRunRef.current) break;
        const next = await getMediaStatus({
          provider_id: current.provider_id,
          id: current.id,
          model: current.model,
          response_url: current.urls.response,
          status_url: current.urls.status,
          kind: current.kind as MediaKind,
        });
        if (run !== generationRunRef.current) break;
        current = next;
        setResults((items) => items.map((item) => (
          item.provider_id === next.provider_id && item.id === next.id ? { ...item, ...next } : item
        )));
        void loadLibrary(undefined, false, true);
        if (isTerminalMediaStatus(next.status) || next.media.length > 0) break;
      }
    } catch (e) {
      if (run === generationRunRef.current) {
        setGenerationError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      pollingKeys.current.delete(key);
    }
  }

  async function submit() {
    if (generationInFlightRef.current) return;
    const provider = selectedProvider;
    if (!provider) {
      setGenerationPhase("failed");
      setGenerationError("Add an enabled Replicate, fal, or OpenRouter media provider first.");
      return;
    }
    if (!model.trim()) {
      setGenerationPhase("failed");
      setGenerationError("Choose a media model before generating.");
      return;
    }
    if (
      modelsLoading ||
      schemaLoading ||
      (metadataProvider && (!modelSchema || !modelOptions.some((item) => item.id === model)))
    ) {
      setGenerationPhase("failed");
      setGenerationError("Wait for the selected model and its settings to finish loading.");
      return;
    }
    if (!prompt.trim()) {
      setGenerationPhase("failed");
      setGenerationError("Write a prompt before generating.");
      return;
    }
    let input: Record<string, unknown>;
    try {
      input = inputWithSchemaControls(advanced, metadataProvider ? modelSchema : null, parameterValues);
    } catch (e) {
      setGenerationPhase("failed");
      setGenerationError(e instanceof Error ? e.message : String(e));
      return;
    }

    const run = ++generationRunRef.current;
    generationInFlightRef.current = true;
    setGenerationPhase("submitting");
    setGenerationError(null);
    setStageRequest({ kind, model: model.trim(), prompt: prompt.trim() });
    setSelectedLibraryId("");
    setResults([]);
    setStageVariantIndex(0);
    try {
      const result = await generateMedia({
        provider_id: provider.id,
        kind,
        model: model.trim(),
        prompt: prompt.trim(),
        input,
      });
      if (run !== generationRunRef.current) return;
      setResults([result]);
      setGenerationPhase("idle");
      if (result.library_id) setSelectedLibraryId(result.library_id);
      void loadLibrary(undefined, false, true);
      void pollMediaStatus(result, run);
    } catch (e) {
      if (run === generationRunRef.current) {
        setGenerationPhase("failed");
        setGenerationError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      generationInFlightRef.current = false;
    }
  }

  function reuseLibraryItem(item: MediaLibraryItem) {
    const provider = available.find((candidate) => candidate.id === item.provider_id);
    const input = structuredClone(item.input ?? {});
    const sameIdentity = Boolean(
      provider &&
      selectedProvider?.id === provider.id &&
      item.kind === kind &&
      item.model === model,
    );
    const reuseWithCurrentSchema = sameIdentity ? modelSchema : null;
    const catalogChanged = provider?.id !== selectedProvider?.id || item.kind !== kind;
    reusedModelRef.current = catalogChanged ? item.model : null;
    reusedInputRef.current = reuseWithCurrentSchema ? null : {
      providerId: item.provider_id,
      model: item.model,
      kind: item.kind,
      input,
    };
    preserveProviderDraftRef.current = false;
    setKind(item.kind);
    setPrompt(item.prompt);
    if (reuseWithCurrentSchema) {
      applyReusedInput(reuseWithCurrentSchema, input);
    } else {
      setAdvanced(JSON.stringify(input, null, 2));
      setParameterValues({});
    }
    resetGenerationFeedback();
    setLibraryActionError(null);
    setLibraryNotice({ id: item.id, label: item.prompt, message: "Settings loaded." });
    if (provider) {
      if (!sameIdentity) {
        setModelSchema(null);
        setSchemaLoading(supportsMediaMetadataProvider(provider));
      } else if (!modelSchema && !schemaLoading) {
        setSchemaLoading(true);
        setSchemaVersion((version) => version + 1);
      }
      if (catalogChanged) {
        setModelOptions([]);
        setModelsLoading(supportsMediaMetadataProvider(provider));
      }
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
      setModelOptions([]);
      setModelSchema(null);
      setModelsLoading(false);
      setSchemaLoading(false);
      setGenerationError(`The original provider ${item.provider} is unavailable. Choose another provider before generating.`);
    }
  }

  async function refreshSelected() {
    if (!selectedLibraryItem) return;
    const id = selectedLibraryItem.id;
    const label = selectedLibraryItem.prompt;
    setLibraryAction({ id, action: "refresh" });
    setLibraryActionError(null);
    setLibraryNotice(null);
    try {
      const next = await refreshMediaLibraryItem(id);
      setLibraryItems((items) => items.map((item) => item.id === next.id ? next : item));
      setLibraryNotice({ id, label, message: "Library item refreshed." });
      window.setTimeout(
        () => void loadLibrary(undefined, false, true),
        next.save_state === "saving" ? 800 : 0,
      );
    } catch (e) {
      setLibraryActionError({ id, label, message: e instanceof Error ? e.message : String(e) });
    } finally {
      setLibraryAction((current) => current?.id === id ? null : current);
    }
  }

  async function deleteSelected() {
    if (!selectedLibraryItem) return;
    const id = selectedLibraryItem.id;
    const label = selectedLibraryItem.prompt;
    if (confirmDeleteId !== id) {
      if (deleteConfirmTimerRef.current !== null) window.clearTimeout(deleteConfirmTimerRef.current);
      setConfirmDeleteId(id);
      setLibraryActionError(null);
      setLibraryNotice(null);
      deleteConfirmTimerRef.current = window.setTimeout(() => {
        setConfirmDeleteId((current) => current === id ? "" : current);
        deleteConfirmTimerRef.current = null;
      }, 3000);
      return;
    }
    if (deleteConfirmTimerRef.current !== null) window.clearTimeout(deleteConfirmTimerRef.current);
    deleteConfirmTimerRef.current = null;
    setConfirmDeleteId("");
    const itemIndex = libraryItems.findIndex((item) => item.id === id);
    setLibraryAction({ id, action: "delete" });
    setLibraryActionError(null);
    setLibraryNotice(null);
    try {
      await deleteMediaLibraryItem(id);
      libraryRequest.current += 1;
      libraryLoadingRef.current = false;
      setLibraryLoading(false);
      const remaining = libraryItems.filter((item) => item.id !== id);
      setConfirmDeleteId("");
      setLibraryItems(remaining);
      if (selectedLibraryIdRef.current === id) {
        const adjacent = remaining[Math.min(Math.max(itemIndex, 0), remaining.length - 1)] ?? null;
        setSelectedLibraryId(adjacent?.id ?? "");
        window.requestAnimationFrame(() => {
          const adjacentCard = adjacent ? libraryCardRefs.current.get(adjacent.id) : null;
          (adjacentCard ?? libraryToggleRef.current)?.focus();
        });
      }
      setLibraryNotice({ id, label, message: "Deleted from local library." });
      void loadLibrary(undefined, false, true);
    } catch (e) {
      setConfirmDeleteId("");
      setLibraryActionError({ id, label, message: e instanceof Error ? e.message : String(e) });
    } finally {
      setLibraryAction((current) => current?.id === id ? null : current);
    }
  }

  async function revealSelected() {
    if (!selectedLibraryItem) return;
    const id = selectedLibraryItem.id;
    const label = selectedLibraryItem.prompt;
    const index = Math.min(stageVariantIndex, Math.max(0, selectedLibraryItem.media.length - 1));
    const path = selectedLibraryItem.media[index]?.local_path;
    if (!path) return;
    setLibraryAction({ id, action: "reveal" });
    setLibraryActionError(null);
    setLibraryNotice(null);
    try {
      await openArtifactLocation(path, "folder");
      setLibraryNotice({ id, label, message: "Opened output location." });
    } catch (e) {
      setLibraryActionError({ id, label, message: e instanceof Error ? e.message : String(e) });
    } finally {
      setLibraryAction((current) => current?.id === id ? null : current);
    }
  }

  function onPromptKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (
      !(event.ctrlKey || event.metaKey) ||
      event.key !== "Enter" ||
      event.nativeEvent.isComposing ||
      event.nativeEvent.keyCode === 229
    ) return;
    event.preventDefault();
    if (canGenerate) void submit();
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

  function panelWidth(panel: MediaSidePanel) {
    const selector = panel === "composer" ? ".media-create-pane" : ".media-library";
    const fallback = panel === "composer" ? composerWidth : libraryWidth;
    return mediaGridRef.current?.querySelector<HTMLElement>(selector)?.getBoundingClientRect().width || fallback;
  }

  function setPanelWidth(panel: MediaSidePanel, width: number) {
    if (panel === "composer") setComposerWidth(width);
    else setLibraryWidth(width);
  }

  function startPanelResize(panel: MediaSidePanel, event: ReactPointerEvent<HTMLDivElement>) {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    resizeCleanupRef.current?.();

    const target = event.currentTarget;
    const originX = event.clientX;
    const originWidth = panelWidth(panel);
    let latestWidth = originWidth;
    let snappedClosed = false;
    let resumeTimer: number | null = null;
    setResizingPanel(panel);
    target.setPointerCapture(event.pointerId);

    const cleanup = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      if (resumeTimer !== null) window.clearTimeout(resumeTimer);
      resizeCleanupRef.current = null;
    };
    const move = (moveEvent: PointerEvent) => {
      if (moveEvent.pointerId !== event.pointerId) return;
      const delta = moveEvent.clientX - originX;
      const rawWidth = originWidth + (panel === "composer" ? delta : -delta);
      if (rawWidth < MIN_SIDEBAR_WIDTH - MEDIA_PANEL_COLLAPSE_OVERSHOOT) {
        if (!snappedClosed) {
          snappedClosed = true;
          if (resumeTimer !== null) window.clearTimeout(resumeTimer);
          resumeTimer = null;
          setResizingPanel(null);
          if (panel === "composer") setComposerPlacement("bottom");
          else setLibraryOpen(false);
        }
        return;
      }
      if (snappedClosed) {
        snappedClosed = false;
        if (panel === "composer") setComposerPlacement("side");
        else setLibraryOpen(true);
        resumeTimer = window.setTimeout(() => {
          setResizingPanel(panel);
          resumeTimer = null;
        }, MEDIA_PANEL_SNAP_ANIMATION_MS);
      }
      latestWidth = normalizeSidebarWidth(rawWidth);
      mediaGridRef.current?.style.setProperty(
        panel === "composer" ? "--media-composer-width" : "--media-library-width",
        `${latestWidth}px`,
      );
      target.setAttribute("aria-valuenow", String(latestWidth));
    };
    const end = (endEvent: PointerEvent) => {
      if (endEvent.pointerId !== event.pointerId) return;
      cleanup();
      if (latestWidth !== originWidth) setPanelWidth(panel, latestWidth);
      setResizingPanel(null);
      if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
    };

    resizeCleanupRef.current = cleanup;
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
  }

  function resizePanelWithKeyboard(panel: MediaSidePanel, event: KeyboardEvent<HTMLDivElement>) {
    const current = panelWidth(panel);
    const direction = panel === "composer" ? 1 : -1;
    let next: number | null = null;
    if (event.key === "ArrowLeft") next = current - MEDIA_PANEL_KEYBOARD_STEP * direction;
    if (event.key === "ArrowRight") next = current + MEDIA_PANEL_KEYBOARD_STEP * direction;
    if (event.key === "Home") next = MIN_SIDEBAR_WIDTH;
    if (event.key === "End") next = MAX_SIDEBAR_WIDTH;
    if (event.key === "Enter") next = panel === "composer" ? DEFAULT_MEDIA_COMPOSER_WIDTH : DEFAULT_MEDIA_LIBRARY_WIDTH;
    if (next === null) return;
    event.preventDefault();
    setPanelWidth(panel, next);
  }

  const stageItems = selectedLibraryItem?.media ?? latestResult?.media ?? [];
  const stageSourceKey = selectedLibraryItem
    ? `library:${selectedLibraryItem.id}`
    : latestResult
      ? latestResult.library_id
        ? `library:${latestResult.library_id}`
        : `result:${latestResult.provider_id}:${latestResult.id}`
      : generationPhase;
  const activeStageRequest = !selectedLibraryItem && !latestResult && generationPhase !== "idle"
    ? stageRequest
    : null;
  const stageModel = selectedLibraryItem?.model ?? latestResult?.model ?? activeStageRequest?.model ?? model;
  const stageKind = selectedLibraryItem?.kind ?? latestResult?.kind ?? activeStageRequest?.kind ?? kind;
  const stagePrompt = selectedLibraryItem?.prompt ?? stageRequest?.prompt ?? "";
  const stageStatus = selectedLibraryItem?.save_state
    ?? latestResult?.save_state
    ?? (latestResult && !isTerminalMediaStatus(latestResult.status) ? "running" : null)
    ?? (generationPhase === "submitting" ? "running" : generationPhase === "failed" ? "failed" : null);
  const selectedVariantIndex = stageItems.length
    ? Math.min(stageVariantIndex, stageItems.length - 1)
    : 0;
  const stageMedia = stageItems[selectedVariantIndex];
  const stageKindLabel = stageKind === "music" ? "audio" : stageKind;
  const stageEmptyTitle = stageStatus === "running"
    ? `Generating ${stageKindLabel}...`
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
  const hasLibraryFilters = Boolean(libraryQuery.trim() || libraryKind || libraryProvider || libraryStatus);
  const modelKindLabel = kind === "music" ? "audio" : kind;
  const modelKindWithArticle = `${/^[aeiou]/.test(modelKindLabel) ? "an" : "a"} ${modelKindLabel}`;
  const mediaKindSettingsLabel = `${modelKindLabel[0].toUpperCase()}${modelKindLabel.slice(1)} settings`;
  const MediaSettingsIcon = kind === "music" ? Volume2 : kind === "video" ? Sparkles : Image;
  const mediaSettingsLabel = selectedProvider
    ? `${selectedProvider.name} · ${model || `Choose ${modelKindWithArticle} model`}`
    : "Add a media provider";
  const selectedProviderAvailable = Boolean(selectedProvider);
  const selectedModelAvailable = metadataProvider
    ? !modelsLoading && modelOptions.some((item) => item.id === model)
    : Boolean(model.trim());
  const canGenerate = Boolean(
    prompt.trim()
    && selectedProviderAvailable
    && selectedModelAvailable
    && model.trim()
    && !modelsLoading
    && !schemaLoading
    && (!metadataProvider || Boolean(modelSchema))
    && !busy
  );
  const loadedCount = `${libraryItems.length}${libraryCursor ? "+" : ""}`;
  const loadedCountDescription = libraryCursor
    ? `${libraryItems.length} loaded, more available`
    : `${libraryItems.length} loaded`;
  const selectedLibraryBusy = libraryAction && libraryAction.id === selectedLibraryItem?.id
    ? libraryAction.action
    : null;
  const deleteConfirmationArmed = confirmDeleteId === selectedLibraryItem?.id;
  const mediaSheetStyle = {
    width: studioSize.width,
    height: studioSize.height,
  } satisfies CSSProperties;
  const mediaGridStyle = {
    "--media-composer-width": `${composerWidth}px`,
    "--media-library-width": `${libraryWidth}px`,
  } as CSSProperties;

  useEffect(() => {
    setStageVariantIndex(0);
  }, [stageSourceKey]);

  useEffect(() => {
    setStageVariantIndex((current) => Math.min(current, Math.max(0, stageItems.length - 1)));
    variantRefs.current.length = stageItems.length;
  }, [stageItems.length]);

  function selectVariant(index: number, focus = false) {
    if (!stageItems.length) return;
    const next = Math.max(0, Math.min(stageItems.length - 1, index));
    setStageVariantIndex(next);
    if (focus) window.requestAnimationFrame(() => variantRefs.current[next]?.focus());
  }

  function onVariantKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    let next: number | null = null;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") next = (index + 1) % stageItems.length;
    if (event.key === "ArrowLeft" || event.key === "ArrowUp") next = (index - 1 + stageItems.length) % stageItems.length;
    if (event.key === "Home") next = 0;
    if (event.key === "End") next = stageItems.length - 1;
    if (next === null) return;
    event.preventDefault();
    selectVariant(next, true);
  }

  if (providersOpen) {
    return (
      <ProvidersManager
        onClose={() => {
          setProvidersOpen(false);
          void refreshProviders(true);
        }}
      />
    );
  }

  const mediaComposer = (
    <section className="media-form media-create-pane" aria-label="Media generator" aria-busy={busy}>
      <ComposerSurface className="media-composer-surface">
        <div className="media-create-controls">
          <div className="control-bar media-control-bar">
            <div className="chips">
              <div className="chip-wrap media-model-picker-wrap">
                <button
                  ref={modelPickerTriggerRef}
                  className="chip chip-model media-model-picker-trigger"
                  data-testid="media-model-picker-trigger"
                  type="button"
                  title={mediaSettingsLabel}
                  aria-label={selectedProvider
                    ? `Choose ${modelKindWithArticle} model${selectedModelLabel ? `, current model ${selectedModelLabel}` : ""}`
                    : "Add media provider"}
                  aria-haspopup={selectedProvider ? "dialog" : undefined}
                  aria-expanded={selectedProvider ? modelPickerOpen : undefined}
                  onClick={selectedProvider ? toggleMediaModelPicker : () => {
                    setMediaSettingsOpen(false);
                    setProvidersOpen(true);
                  }}
                >
                  <span className={`dot ${selectedModelAvailable ? "dot-green" : "dot-yellow"}`} />
                  <span className="chip-label">
                    {selectedProvider
                      ? modelsLoading ? `Loading ${modelKindLabel} models...` : selectedModelLabel || `Choose ${modelKindWithArticle} model`
                      : "Add media provider"}
                  </span>
                  {selectedProvider && <ChevronDown size={12} className="chip-chev" />}
                </button>
              </div>
              <div className="control-inline-slot">
                <button
                  className={`chip media-settings-toggle${mediaSettingsOpen ? " active" : ""}`}
                  data-testid="inline-media-settings-summary"
                  type="button"
                  aria-label={mediaKindSettingsLabel}
                  aria-controls="media-inline-settings"
                  aria-expanded={mediaSettingsOpen}
                  onClick={() => {
                    if (!mediaSettingsOpen) closeModelPicker(false);
                    setMediaSettingsOpen((open) => !open);
                  }}
                >
                  <MediaSettingsIcon size={13} />
                  <span className="chip-label">{mediaKindSettingsLabel}</span>
                </button>
              </div>
              <div id="media-inline-settings" className="media-inline-settings" hidden={!mediaSettingsOpen}>
                <InlineMediaControls
                  providerName={selectedProvider?.name || "Media"}
                  model={model}
                  kind={kind}
                  supportedKinds={["image", "video", "music"]}
                  schema={modelSchema}
                  schemaLoading={schemaLoading}
                  parameterValues={parameterValues}
                  advanced={advanced}
                  error={null}
                  onKindChange={(nextKind) => {
                    preserveProviderDraftRef.current = false;
                    reusedInputRef.current = null;
                    setKind(nextKind);
                    setModel("");
                    setModelOptions([]);
                    setModelSchema(null);
                    setSchemaLoading(false);
                    setModelsLoading(Boolean(selectedProvider && metadataProvider));
                    resetGenerationFeedback();
                  }}
                  onParameterChange={updateParameter}
                  onAdvancedChange={updateAdvanced}
                />
              </div>
            </div>
          </div>

          {generationError && (
            <div className="artifact-error media-generator-alert" data-testid="media-generation-error" role="alert">
              {generationError}
            </div>
          )}
        </div>

        <div className="composer comfortable media-composer-box">
          <div className="composer-input-wrap">
            <textarea
              ref={promptRef}
              className="composer-input media-composer-prompt"
              data-testid="media-prompt-input"
              value={prompt}
              rows={1}
              dir="auto"
              aria-label="Media prompt"
              placeholder={kind === "music" ? "Warm instrumental synthwave with a steady pulse..." : "Product photo on a clean workbench, natural side light..."}
              onChange={(e) => {
                setPrompt(e.target.value);
                resetGenerationFeedback();
              }}
              onKeyDown={onPromptKeyDown}
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
                disabled={!canGenerate}
                onClick={() => void submit()}
              >
                <ArrowUp size={17} />
              </button>
            </div>
          </div>
        </div>
      </ComposerSurface>
    </section>
  );

  return (
    <SheetDialog
      title="Media studio"
      className="sheet media-sheet"
      testId="media-generator"
      style={mediaSheetStyle}
      onClose={onClose}
    >
      <div className="media-studio">
        <div className="sheet-header media-studio-header">
          <h2>Media studio</h2>
          <button className="icon-btn" type="button" onClick={onClose} title="Close" aria-label="Close media studio">
            <X size={15} />
          </button>
        </div>

        <div
          ref={mediaGridRef}
          className={`media-grid${libraryOpen ? " library-open" : ""}${resizingPanel ? ` resizing-${resizingPanel}` : ""}`}
          style={mediaGridStyle}
        >
          <section className="media-stage" data-testid="media-stage" aria-label="Generated media">
              <div className="media-stage-head">
                <div className="media-stage-title">
                  <strong>Output</strong>
                  <span title={stageModel}>{stageModel || "Nothing selected"}</span>
                  {stageItems.length > 1 && (
                    <span className="media-output-count">{stageItems.length} outputs</span>
                  )}
                </div>
                <div className="media-stage-head-actions">
                  {stageStatus && stageStatus !== "ready" && <span className={`media-status ${stageStatus}`} role="status" aria-live="polite">{stageStatus}</span>}
                  <div className="media-composer-placement" role="group" aria-label="Composer placement" data-testid="media-composer-placement">
                    <button
                      type="button"
                      aria-pressed={composerPlacement === "side"}
                      onClick={() => setComposerPlacement("side")}
                    >
                      Side
                    </button>
                    <button
                      type="button"
                      aria-pressed={composerPlacement === "bottom"}
                      onClick={() => setComposerPlacement("bottom")}
                    >
                      Bottom
                    </button>
                  </div>
                  <button
                    ref={libraryToggleRef}
                    className={`btn-ghost media-library-toggle${libraryOpen ? " active" : ""}`}
                    type="button"
                    aria-label={`${libraryOpen ? "Close" : "Open"} local library, ${loadedCountDescription}`}
                    aria-controls="media-library-sidebar"
                    aria-expanded={libraryOpen}
                    onClick={() => {
                      const nextOpen = !libraryOpen;
                      setLibraryOpen(nextOpen);
                      if (nextOpen && !libraryItems.length && !libraryLoading) void loadLibrary();
                    }}
                  >
                    <Sidebar size={14} />
                    <span>Library</span>
                    <span className="media-library-count" aria-hidden="true">{loadedCount}</span>
                  </button>
                </div>
              </div>
              <div className={`media-output-body ${composerPlacement}`} data-composer-placement={composerPlacement}>
                <div className={`media-stage-preview${stageItems.length ? " has-media" : ""}`} aria-busy={busy}>
                  {stageItems.length && stageKind === "music" ? (
                    <div className="media-stage-audio-list">
                      {stageItems.map((item, index) => (
                        <GeneratedMedia
                          key={`${item.url}:${index}`}
                          item={item}
                          alt={`Generated audio ${index + 1} of ${stageItems.length} from ${stageModel}`}
                          onOpenExternal={(url) => void openExternalUrl(url)}
                        />
                      ))}
                    </div>
                  ) : stageMedia ? (
                    <GeneratedMedia
                      item={stageMedia}
                      alt={`Generated ${stageKind} ${selectedVariantIndex + 1} of ${stageItems.length} from ${stageModel}`}
                      onOpenExternal={(url) => void openExternalUrl(url)}
                    />
                  ) : stageStatus === "running" || stageStatus === "saving" ? (
                    <div
                      className={`media-generation-placeholder ${stageStatus}`}
                      data-testid={stageStatus === "running" ? "media-generation-progress" : undefined}
                      role="status"
                      aria-live="polite"
                    >
                      <div className="media-generation-field" aria-hidden="true" />
                      <div className="media-generation-copy">
                        <strong className={stageStatus === "running" ? "shiny-text" : undefined}>{stageEmptyTitle}</strong>
                        <span>{stageStatus === "running" && stagePrompt ? stagePrompt : stageEmptyDetail}</span>
                      </div>
                    </div>
                  ) : (
                    <div className="media-empty">
                      <Image size={28} />
                      <strong>{stageEmptyTitle}</strong>
                      <span>{stageEmptyDetail}</span>
                    </div>
                  )}
                </div>
                {mediaComposer}
                {composerPlacement === "side" && (
                  <PaneResizeHandle
                    className={`media-panel-resize-handle media-composer-resize-handle${resizingPanel === "composer" ? " dragging" : ""}`}
                    orientation="vertical"
                    data-testid="media-composer-resize-handle"
                    aria-label="Resize media composer; drag past minimum to move it below Output"
                    aria-valuemin={MIN_SIDEBAR_WIDTH}
                    aria-valuemax={MAX_SIDEBAR_WIDTH}
                    aria-valuenow={composerWidth}
                    tabIndex={0}
                    onPointerDown={(event) => startPanelResize("composer", event)}
                    onKeyDown={(event) => resizePanelWithKeyboard("composer", event)}
                    onDoubleClick={() => setComposerWidth(DEFAULT_MEDIA_COMPOSER_WIDTH)}
                  />
                )}
              </div>
              {stageKind !== "music" && stageItems.length > 1 && (
                <div className="media-variant-strip" data-testid="media-variant-strip" role="listbox" aria-label="Output variants">
                  {stageItems.map((item, index) => (
                    <button
                      ref={(node) => {
                        variantRefs.current[index] = node;
                      }}
                      className={selectedVariantIndex === index ? "active" : ""}
                      data-testid="media-variant"
                      key={`${item.url}:${index}`}
                      type="button"
                      role="option"
                      aria-label={`Show variant ${index + 1} of ${stageItems.length}`}
                      aria-selected={selectedVariantIndex === index}
                      tabIndex={selectedVariantIndex === index ? 0 : -1}
                      onClick={() => selectVariant(index)}
                      onKeyDown={(event) => onVariantKeyDown(event, index)}
                    >
                      <GeneratedMedia
                        item={item}
                        alt={`Variant ${index + 1} of ${stageItems.length}`}
                        interactive={false}
                        pressed={selectedVariantIndex === index}
                      />
                      <span className="media-variant-number">{index + 1}</span>
                    </button>
                  ))}
                </div>
              )}
              {selectedLibraryItem && (
                <div className="media-stage-meta">
                  <p>{selectedLibraryItem.prompt}</p>
                  <div className="media-stage-actions" aria-busy={Boolean(selectedLibraryBusy)}>
                    <button className="btn-ghost" type="button" disabled={Boolean(selectedLibraryBusy)} onClick={() => reuseLibraryItem(selectedLibraryItem)}>Use settings</button>
                    {(selectedLibraryItem.save_state === "running" || selectedLibraryItem.save_state === "failed") && (
                      <button className="btn-ghost" type="button" disabled={Boolean(selectedLibraryBusy)} onClick={() => void refreshSelected()}>
                        <Refresh size={13} /> {selectedLibraryBusy === "refresh" ? "Refreshing..." : "Refresh"}
                      </button>
                    )}
                    {selectedLibraryItem.media[selectedVariantIndex]?.local_path && (
                      <button className="btn-ghost" type="button" disabled={Boolean(selectedLibraryBusy)} onClick={() => void revealSelected()}>
                        <FolderOpen size={13} /> {selectedLibraryBusy === "reveal" ? "Opening..." : "Reveal"}
                      </button>
                    )}
                    <button
                      className="btn-ghost danger"
                      type="button"
                      aria-describedby={deleteConfirmationArmed && !selectedLibraryBusy ? "media-delete-confirmation" : undefined}
                      disabled={Boolean(selectedLibraryBusy)}
                      onClick={() => void deleteSelected()}
                    >
                      {confirmDeleteId === selectedLibraryItem.id ? <Check size={13} /> : <Trash size={13} />}
                      {selectedLibraryBusy === "delete" ? "Deleting..." : confirmDeleteId === selectedLibraryItem.id ? "Confirm delete" : "Delete"}
                    </button>
                  </div>
                  {deleteConfirmationArmed && !selectedLibraryBusy && (
                    <div id="media-delete-confirmation" className="media-delete-confirmation" role="status">
                      Delete again within 3 seconds to permanently remove this item and its local files.
                    </div>
                  )}
                  {selectedLibraryItem.error && <div className="artifact-error" role="alert">{selectedLibraryItem.error}</div>}
                  {libraryActionError?.id === selectedLibraryItem.id && (
                    <div className="artifact-error media-library-action-error" role="alert">{libraryActionError.message}</div>
                  )}
                  {libraryNotice?.id === selectedLibraryItem.id && (
                    <div className="media-library-notice" role="status" aria-live="polite">{libraryNotice.message}</div>
                  )}
                </div>
              )}
              {libraryActionError && libraryActionError.id !== selectedLibraryItem?.id && (
                <div className="artifact-error media-library-global-feedback" role="alert">
                  {libraryActionError.label}: {libraryActionError.message}
                </div>
              )}
              {libraryNotice && libraryNotice.id !== selectedLibraryItem?.id && (
                <div className="media-library-notice media-library-global-feedback" role="status" aria-live="polite">
                  {libraryNotice.label}: {libraryNotice.message}
                </div>
              )}
          </section>

          {libraryOpen && <aside className="media-library" id="media-library-sidebar" aria-label="Local library">
              <PaneResizeHandle
                className={`media-panel-resize-handle media-library-resize-handle${resizingPanel === "library" ? " dragging" : ""}`}
                orientation="vertical"
                data-testid="media-library-resize-handle"
                aria-label="Resize local library; drag past minimum to close it"
                aria-valuemin={MIN_SIDEBAR_WIDTH}
                aria-valuemax={MAX_SIDEBAR_WIDTH}
                aria-valuenow={libraryWidth}
                tabIndex={0}
                onPointerDown={(event) => startPanelResize("library", event)}
                onKeyDown={(event) => resizePanelWithKeyboard("library", event)}
                onDoubleClick={() => setLibraryWidth(DEFAULT_MEDIA_LIBRARY_WIDTH)}
              />
              <div className="media-library-head">
                <div>
                  <span className="media-eyebrow">Local library</span>
                  <strong>{loadedCount} loaded</strong>
                  {libraryLoading && libraryItems.length > 0 && (
                    <span className="media-library-updating" role="status">Updating...</span>
                  )}
                </div>
                <div className="media-library-search">
                  <Search size={13} aria-hidden="true" />
                  <input value={libraryQuery} aria-label="Search media library" placeholder="Search prompts or models..." onChange={(e) => setLibraryQuery(e.target.value)} />
                </div>
              </div>
              {showLibraryFilters && (
                <div className="media-library-filters">
                  <div className="media-filter-tabs" role="group" aria-label="Filter library by media type">
                    {(["", "image", "video", "music"] as const).map((value) => (
                      <button key={value || "all"} type="button" className={libraryKind === value ? "active" : ""} aria-pressed={libraryKind === value} onClick={() => setLibraryKind(value)}>
                        {value || "All"}
                      </button>
                    ))}
                  </div>
                  <Select
                    value={libraryProvider}
                    placeholder="All providers"
                    ariaLabel="Provider filter"
                    options={[{ label: "All providers", value: "" }, ...available.map((provider) => ({ label: provider.name, value: provider.id }))]}
                    onChange={setLibraryProvider}
                  />
                  <Select
                    value={libraryStatus}
                    placeholder="Any status"
                    ariaLabel="Status filter"
                    options={[
                      { label: "Any status", value: "" },
                      { label: "Ready", value: "ready" },
                      { label: "Running", value: "running" },
                      { label: "Saving", value: "saving" },
                      { label: "Failed", value: "failed" },
                    ]}
                    onChange={(value) => setLibraryStatus(value as MediaLibraryStatus | "")}
                  />
                  {hasLibraryFilters && (
                    <button
                      className="btn-ghost media-clear-filters"
                      type="button"
                      onClick={() => {
                        setLibraryQuery("");
                        setLibraryKind("");
                        setLibraryProvider("");
                        setLibraryStatus("");
                      }}
                    >
                      Clear filters
                    </button>
                  )}
                </div>
              )}

              {libraryListError && (
                <div className="artifact-error media-library-list-error" role="alert">
                  <span>{libraryListError}</span>
                  <button className="btn-ghost" type="button" onClick={() => void loadLibrary()}>Retry</button>
                </div>
              )}
              <div className="media-library-grid" aria-busy={libraryLoading}>
                {libraryItems.map((item) => (
                  <button
                    ref={(node) => {
                      if (node) libraryCardRefs.current.set(item.id, node);
                      else libraryCardRefs.current.delete(item.id);
                    }}
                    className={`media-library-card${item.id === selectedLibraryItem?.id ? " active" : ""}`}
                    data-testid="media-library-item"
                    type="button"
                    aria-label={`Select ${item.kind}: ${item.prompt}, ${item.provider}, ${item.model}, ${item.save_state}${item.media.length > 1 ? `, ${item.media.length} outputs` : ""}`}
                    aria-current={item.id === selectedLibraryItem?.id ? "true" : undefined}
                    key={item.id}
                    onClick={() => {
                      setLibraryNotice(null);
                      setLibraryActionError(null);
                      setSelectedLibraryId(item.id);
                    }}
                  >
                    <span className="media-library-thumb">
                      {item.media[0] ? (
                        <GeneratedMedia
                          item={item.media[0]}
                          alt={`Generated ${item.kind} from ${item.model}`}
                          interactive={false}
                          pressed={item.id === selectedLibraryItem?.id}
                        />
                      ) : (
                        <Image size={20} />
                      )}
                      {item.save_state !== "ready" && <span className={`media-status ${item.save_state}`}>{item.save_state}</span>}
                      {item.media.length > 1 && <span className="media-output-count">{item.media.length} outputs</span>}
                    </span>
                    <span className="media-library-card-body">
                      <strong title={item.prompt}>{item.prompt}</strong>
                      <span className="media-library-card-meta" aria-label={`${item.provider}, ${item.model}`}>
                        <span className="media-library-card-provider">{item.provider}</span>
                        <span className="media-library-card-model" title={item.model}>{item.model.split("/").pop() || item.model}</span>
                      </span>
                    </span>
                  </button>
                ))}
                {!libraryItems.length && libraryLoading && (
                  <div className="media-library-empty" role="status">Loading local library...</div>
                )}
                {!libraryItems.length && !libraryLoading && !libraryListError && (
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
        </div>
        {modelPickerOpen && modelPickerStyle && (
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
              onClose={closeModelPicker}
              ariaLabel={`Choose ${modelKindWithArticle} model`}
              showManagementActions={false}
              favoriteIds={favoriteModelIds}
              favoritesOnlyValue={favoritesOnly}
              onToggleFavorite={toggleModelFavorite}
              onFavoritesOnlyChange={setFavoritesOnly}
              searchPlaceholder={`Search ${kind === "music" ? "audio" : kind} models...`}
              emptyMessage={`No ${kind === "music" ? "audio" : kind} models available.`}
            />
          </div>
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
