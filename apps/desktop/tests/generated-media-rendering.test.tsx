import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createServer } from "vite";
import type { MediaResultItem } from "../src/api.js";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

type GeneratedMediaProps = {
  item?: MediaResultItem | null;
  alt: string;
  onOpenExternal?: (url: string) => void;
  onActivate?: () => void;
  interactive?: boolean;
  pressed?: boolean;
};

const server = await createServer({
  root: process.cwd(),
  appType: "custom",
  logLevel: "silent",
  server: { middlewareMode: true },
});

try {
  const { GeneratedMedia } = (await server.ssrLoadModule(
    "/src/components/GeneratedMedia.tsx",
  )) as { GeneratedMedia: ComponentType<GeneratedMediaProps> };
  const { MediaManager } = (await server.ssrLoadModule(
    "/src/components/MediaManager.tsx",
  )) as {
    MediaManager: ComponentType<{ onClose: () => void }>;
  };
  const { InlineMediaControls } = (await server.ssrLoadModule(
    "/src/components/InlineMediaControls.tsx",
  )) as {
    InlineMediaControls: ComponentType<Record<string, unknown>>;
  };
  const { Select } = (await server.ssrLoadModule(
    "/src/components/ui.tsx",
  )) as {
    Select: ComponentType<{
      value: string;
      options: Array<{ value: string; label: string }>;
      onChange: (value: string) => void;
      ariaLabel?: string;
    }>;
  };
  const { inputWithSchemaControls } = (await server.ssrLoadModule(
    "/src/lib/media.ts",
  )) as {
    inputWithSchemaControls: (
      advanced: string,
      schema: Record<string, unknown> | null,
      values: Record<string, unknown>,
    ) => Record<string, unknown>;
  };

  const manager = renderToStaticMarkup(
    createElement(MediaManager, { onClose: () => {} }),
  );
  assert(manager.includes('data-testid="media-generator"'), "The standalone media manager should remain reachable");
  assert(manager.includes('data-testid="inline-media-generator"'), "The media manager should expose the chat media controls");
  assert(manager.includes('data-testid="media-stage"'), "The studio should expose a dedicated output stage");
  assert(manager.includes('aria-controls="media-library-sidebar"'), "The studio should expose a local-library sidebar toggle");
  assert(manager.includes('aria-expanded="false"'), "The local library should start closed");
  assert(!manager.includes('id="media-library-sidebar"'), "The closed library should not leave a hidden sidebar in the accessibility tree");
  assert(manager.includes('data-testid="inline-media-advanced-input"'), "Raw media input should reuse the chat composer disclosure");
  assert(manager.includes("media-composer-dock"), "The standalone generator should use a full-width composer dock");
  assert(manager.includes("dock-surface media-composer-surface"), "The prompt should use the shared chat composer surface");
  assert(manager.includes("composer-input media-composer-prompt"), "The media prompt should use the chat composer input structure");
  assert(manager.includes("control-bar media-control-bar"), "Model selection should use the chat composer control bar");
  assert(manager.includes('data-testid="media-model-picker-trigger"'), "The composer model chip should be the model picker entry point");
  assert(manager.includes('aria-label="Add media provider"'), "The empty provider state should make setup the model chip's primary action");
  assert(manager.includes(">Add media provider</span>"), "The no-provider setup action should be visible, not only announced");
  assert(manager.includes('data-testid="inline-media-generator"'), "Generation parameters should reuse the chat media controls");
  assert(manager.includes("send-btn media-composer-send-btn"), "Generation should reuse the chat send-button treatment");
  assert(manager.includes('aria-label="Generate image"'), "The icon-only generation action should retain an accessible label");
  assert(manager.includes('data-testid="media-studio-resize-handle"'), "The studio should expose a keyboard-operable resize handle");
  assert(!manager.includes("media-sheet-resize-glyph"), "The resize handle should not render a panel-specific glyph");
  assert(manager.includes("Ctrl/Cmd + Enter"), "The generation shortcut should be shown outside the primary action label");
  assert(manager.includes('dir="auto"'), "The media prompt should infer left-to-right or right-to-left direction");
  assert(manager.includes("Generate, compare outputs, and reuse saved settings."), "The studio should describe its complete quick-generation workflow");
  assert(!manager.includes("Iteration stays in chat"), "The studio should not hand users off to chat for comparison or reuse");
  assert(manager.includes("Prompt sent unchanged"), "The privacy summary should remain concise in the generator rail");
  assert(manager.includes("Your next output will appear here"), "The initial preview should explain where results appear");
  assert(!manager.includes('data-testid="inline-media-settings-summary"'), "Media Studio should retain inline media controls");

  const namedSelect = renderToStaticMarkup(
    createElement(Select, {
      value: "ready",
      options: [
        { value: "ready", label: "Ready" },
        { value: "failed", label: "Failed" },
      ],
      ariaLabel: "Status filter",
      onChange: () => {},
    }),
  );
  assert(namedSelect.includes('aria-label="Status filter"'), "Shared selects should expose their visible purpose");
  assert(namedSelect.includes('aria-haspopup="listbox"'), "Shared selects should announce listbox popup semantics");
  assert(namedSelect.includes('aria-expanded="false"'), "Closed shared selects should expose their collapsed state");
  assert(namedSelect.includes("aria-controls="), "Shared select triggers should identify their controlled menu");

  const chatMediaControls = renderToStaticMarkup(
    createElement(InlineMediaControls, {
      providerName: "OpenRouter",
      model: "google/gemini-image",
      kind: "image",
      schema: {
        provider_id: "openrouter",
        model: "google/gemini-image",
        kind: "image",
        supported_parameters: ["aspect_ratio"],
        controls: [{ key: "aspect_ratio", label: "Aspect ratio", kind: "select", options: [{ value: "1:1", label: "1:1" }] }],
      },
      schemaLoading: false,
      parameterValues: { aspect_ratio: "1:1" },
      advanced: "{}",
      error: "Schema unavailable",
      popover: true,
      onKindChange: () => {},
      onParameterChange: () => {},
      onAdvancedChange: () => {},
    }),
  );
  assert(chatMediaControls.includes('data-testid="inline-media-settings-summary"'), "Chat media controls should collapse into one settings pill");
  assert(chatMediaControls.includes('aria-label="Image settings, error"'), "The settings pill should expose its media kind and error state");
  assert(chatMediaControls.includes('data-testid="inline-media-param-aspect_ratio"'), "Popover media controls should retain schema parameters");
  assert(chatMediaControls.includes('aria-label="Media type"'), "Media kind selection should have an accessible name");
  assert(chatMediaControls.includes('aria-label="Aspect ratio"'), "Schema controls should use their visible labels as accessible names");
  assert(chatMediaControls.includes('aria-label="Advanced media input JSON"'), "Advanced structured input should have an accessible name");
  assert(chatMediaControls.includes('data-testid="inline-media-advanced-input"'), "Popover media controls should retain Advanced input");
  assert(chatMediaControls.includes('data-testid="inline-media-error"'), "Popover media controls should retain the detailed error");
  assert(chatMediaControls.includes('role="alert"'), "Media control errors should be announced immediately");

  const image = renderToStaticMarkup(
    createElement(GeneratedMedia, {
      item: { kind: "image", url: "https://cdn.example/image.png", mime: "image/png" },
      alt: "Generated image",
      onOpenExternal: () => {},
    }),
  );
  assert(image.includes('data-testid="generated-media-image"'), "Images should render as preview buttons");
  assert(image.includes("cursor") === false, "Rendering should not add inline cursor styles");
  assert(!image.includes("<a "), "Generated images should not navigate away when clicked");

  const activeSelection = renderToStaticMarkup(
    createElement(GeneratedMedia, {
      item: { kind: "image", url: "https://cdn.example/select.png", mime: "image/png" },
      alt: "Show output 2 of 3",
      onActivate: () => {},
      pressed: true,
    }),
  );
  assert(activeSelection.includes('aria-label="Show output 2 of 3"'), "Selectable thumbnails should use the supplied selection label verbatim");
  assert(activeSelection.includes('aria-pressed="true"'), "Selectable thumbnails should expose their selected state");

  const passiveImage = renderToStaticMarkup(
    createElement(GeneratedMedia, {
      item: { kind: "image", url: "https://cdn.example/passive.png", mime: "image/png" },
      alt: "Saved image thumbnail",
      interactive: false,
    }),
  );
  assert(passiveImage.includes('class="generated-media-thumbnail passive"'), "Library thumbnails should support passive rendering");
  assert(passiveImage.includes('role="img"'), "Passive visual thumbnails should retain image semantics");
  assert(!passiveImage.includes("<button"), "Passive visual thumbnails should not add a second focus target");

  const video = renderToStaticMarkup(
    createElement(GeneratedMedia, {
      item: { kind: "video", url: "https://cdn.example/video.mp4", mime: "video/mp4" },
      alt: "Generated video",
    }),
  );
  assert(video.includes('data-testid="generated-media-video"'), "Videos should render as preview buttons");
  assert(video.includes("<video"), "Video thumbnails should render a video frame");
  assert(!video.includes("controls"), "Video thumbnails should stay non-playing and omit controls");

  const music = renderToStaticMarkup(
    createElement(GeneratedMedia, {
      item: { kind: "music", url: "data:audio/mpeg;base64,QUJD", mime: "audio/mpeg" },
      alt: "Generated music",
    }),
  );
  assert(music.includes('data-testid="generated-media-music"'), "Music should use its dedicated inline result");
  assert(music.includes("<audio"), "Music should render native audio controls");
  assert(music.includes("controls"), "Music audio should expose playback controls");
  assert(music.includes('preload="metadata"'), "Music should only preload metadata");

  const passiveMusic = renderToStaticMarkup(
    createElement(GeneratedMedia, {
      item: { kind: "music", url: "data:audio/mpeg;base64,QUJD", mime: "audio/mpeg" },
      alt: "Saved audio thumbnail",
      interactive: false,
    }),
  );
  assert(passiveMusic.includes('class="generated-media-audio passive"'), "Library audio should support passive rendering");
  assert(passiveMusic.includes('role="img"'), "Passive audio should retain named thumbnail semantics");
  assert(!passiveMusic.includes("<audio"), "Passive audio thumbnails should not add native playback focus targets");

  const structuredSchema = {
    model: "fixture/media",
    provider_id: "fixture",
    provider: "Fixture",
    supported_parameters: ["references", "guidance"],
    controls: [
      { key: "references", label: "References", kind: "array", path: ["input", "references"], item_kind: "string" },
      { key: "guidance", label: "Guidance", kind: "json", path: ["input", "guidance"] },
    ],
  };
  const parsedStructuredInput = inputWithSchemaControls(
    '{"seed":7}',
    structuredSchema,
    {
      references: "first.png\nsecond.png",
      guidance: '{"strength":0.75}',
    },
  );
  assert(
    JSON.stringify(parsedStructuredInput) === JSON.stringify({
      seed: 7,
      input: {
        references: ["first.png", "second.png"],
        guidance: { strength: 0.75 },
      },
    }),
    "Submit-time media input assembly should parse raw array and JSON controls",
  );
  const preservedStructuredInput = inputWithSchemaControls(
    "{}",
    structuredSchema,
    {
      references: ["legacy.png"],
      guidance: { strength: 1 },
    },
  );
  assert(
    JSON.stringify(preservedStructuredInput) === JSON.stringify({
      input: {
        references: ["legacy.png"],
        guidance: { strength: 1 },
      },
    }),
    "Submit-time media input assembly should preserve already-structured legacy values",
  );
  let invalidNumberArrayRejected = false;
  try {
    inputWithSchemaControls("{}", {
      ...structuredSchema,
      controls: [{
        key: "references",
        label: "Reference weights",
        kind: "array",
        path: ["input", "references"],
        item_kind: "number",
      }],
    }, { references: "1\nnot-a-number" });
  } catch {
    invalidNumberArrayRejected = true;
  }
  assert(invalidNumberArrayRejected, "Submit-time array parsing should reject invalid numeric entries");

  const source = readFileSync(resolve(process.cwd(), "src/components/GeneratedMedia.tsx"), "utf8");
  assert(source.includes("<SheetDialog"), "Visual media should reuse the focus-trapped dialog");
  assert(source.includes("generated-media-stage"), "Expanded visual media should render in the contained stage");
  assert(source.includes("media-zoom-increase"), "Expanded visual media should expose zoom controls");
  assert(source.includes("media-preview-close"), "Expanded visual media should expose a visible close action");
  assert(source.includes("scrollLeft = start.left"), "Zoomed visual media should support pointer panning");
  assert(source.includes("onWheel={zoomWithWheel}"), "Expanded visual media should zoom with the mouse wheel");
  assert(source.includes("createPortal("), "Chat media previews should escape clipped message-card layout");
  assert(source.includes("closeFromBackdropClick"), "Dark canvas clicks should dismiss the fullscreen preview");
  assert(source.includes("Open externally"), "Direct web media should retain the secondary external action");
  assert(source.includes("onActivate"), "Library thumbnails should support selection without forcing full-screen preview");
  assert(source.includes("interactive = true"), "Generated media should default to the existing interactive preview behavior");
  assert(source.includes('className="generated-media-thumbnail passive"'), "Generated media should expose a passive visual mode");
  assert(source.includes('className="generated-media-audio passive"'), "Generated media should expose a passive audio mode");

  const managerSource = readFileSync(resolve(process.cwd(), "src/components/MediaManager.tsx"), "utf8");
  assert(managerSource.includes("event.ctrlKey || event.metaKey"), "The studio should generate with Ctrl/Cmd+Enter");
  assert(managerSource.includes("onKeyDown={onPromptKeyDown}"), "The generation shortcut should be scoped to the media prompt");
  assert(!managerSource.includes("onKeyDown={onStudioKeyDown}"), "Search and advanced controls should not inherit a studio-wide generation shortcut");
  assert(managerSource.includes("event.nativeEvent.isComposing"), "The prompt shortcut should ignore IME composition");
  assert(managerSource.includes("promptRef"), "The media prompt should retain a direct auto-grow target");
  assert(managerSource.includes("Math.min(textarea.scrollHeight, 150)"), "The media prompt should auto-grow to its existing height cap");
  assert(managerSource.includes("[prompt, providersOpen]"), "The prompt should restore its auto-grown height after provider setup");
  assert(!managerSource.includes("createPortal("), "The model picker should stay inside the studio dialog and focus trap");
  assert(managerSource.includes('aria-live="polite"'), "Generation state should be announced to assistive technology");
  assert(managerSource.includes('"media-generation-progress"'), "A valid submit should expose an immediate stage progress state");
  assert(managerSource.includes('setGenerationPhase("submitting")'), "Generation should enter its busy phase before awaiting the provider");
  assert(managerSource.includes("setResults([])"), "A new valid submit should clear stale stage output before waiting");
  assert(managerSource.includes("generationError"), "Generation failures should remain scoped to the generator");
  assert(managerSource.includes("libraryListError"), "Library-list failures should remain scoped to the library");
  assert(managerSource.includes("libraryActionError"), "Saved-item action failures should remain scoped to their selected item");
  assert(managerSource.includes("(!metadataProvider || Boolean(modelSchema))"), "Generation should stay disabled until metadata-backed schema is ready");
  assert(managerSource.includes("loadLibrary(undefined, false, quiet)"), "The initial library badge load should remain quiet");
  assert(managerSource.includes('data-testid="media-generation-error"'), "Generation errors should have a stable announced surface");
  assert(managerSource.includes("confirmDeleteId"), "Permanent deletion should require a second confirmation action");
  assert(managerSource.includes("window.setTimeout"), "Permanent deletion confirmation should expire automatically");
  assert(managerSource.includes("}, 3000)"), "Permanent deletion should use the documented three-second confirmation window");
  assert(managerSource.includes("Delete again within 3 seconds to permanently remove this item and its local files."), "The timed destructive action should explain its confirmation window and consequence");
  assert(managerSource.includes('"Deleting..."'), "The confirmed destructive action should expose its busy state");
  assert(managerSource.includes("(adjacentCard ?? libraryToggleRef.current)?.focus()"), "Deleting should focus an adjacent card or fall back to the library toggle");
  assert(managerSource.includes("reuseLibraryItem"), "Saved prompt and settings should be reusable");
  assert(managerSource.includes("<ProvidersManager"), "Provider setup should happen in place without unmounting MediaManager state");
  assert(managerSource.includes("void refreshProviders(true)"), "Closing provider setup should refresh providers while preserving the studio draft");
  assert(managerSource.includes("<ModelPicker"), "Media Studio should reuse the chat composer model picker");
  assert(managerSource.includes("<InlineMediaControls"), "Media Studio should reuse the chat media control row");
  assert(managerSource.includes('["image", "video", "music"] as MediaKind[]'), "The shared model picker should discover every media kind");
  assert(managerSource.includes("route.kinds.includes(kind)"), "The selected media type should filter the model picker");
  assert(managerSource.includes("bottom: bounds.bottom - rect.top + gap"), "Short upward-opening model pickers should stay anchored to their trigger");
  assert(!managerSource.includes('testId="media-model-select"'), "The model picker should not duplicate search and selection controls");
  assert(!managerSource.includes('className="field media-model-field"'), "Generation settings should not repeat the model picker");
  assert(managerSource.includes("favoriteIds={favoriteModelIds}"), "The shared picker should retain provider-scoped media favorites");
  assert(managerSource.includes("setMediaStudioSize"), "The resized studio dimensions should be persisted");
  assert(managerSource.includes("Saving locally..."), "The preview should distinguish local saving from generation");
  assert(managerSource.includes("This run failed"), "The preview should present a specific failed state");
  assert(managerSource.includes("Loading local library..."), "Initial library loading should not leave a blank grid");
  assert(managerSource.includes('stageStatus !== "ready"'), "Ready outputs should not repeat their state in the stage header");
  assert(managerSource.includes('className="media-library-count"'), "The loaded library count should render as a distinct count badge");
  assert(managerSource.includes('`${libraryItems.length}${libraryCursor ? "+" : ""}`'), "A loaded page with another cursor should use an accurate plus-suffixed count");
  assert(managerSource.includes('item.save_state !== "ready"'), "Ready library cards should not repeat their state over the thumbnail");
  assert(managerSource.includes('className="media-library-card-meta"'), "Library cards should separate prompt and source metadata");
  assert(managerSource.includes("showLibraryFilters"), "Empty libraries should hide inactive filter chrome");
  assert(managerSource.includes("hasLibraryFilters"), "The library should track whether there is anything to clear");
  assert(managerSource.includes("Clear filters"), "Active library filters should have one reversible reset");
  assert(managerSource.includes('ariaLabel="Provider filter"'), "The provider filter should have an accessible name");
  assert(managerSource.includes('ariaLabel="Status filter"'), "The status filter should have an accessible name");
  assert(managerSource.includes('className={`media-library-card${'), "A library item should use the whole card as its selection control");
  assert(managerSource.includes('data-testid="media-library-item"'), "The single library-card focus target should remain testable");
  assert(managerSource.includes('aria-current={item.id === selectedLibraryItem?.id ? "true" : undefined}'), "The selected library card should expose its current state");
  assert(managerSource.includes("interactive={false}"), "Library and variant thumbnails should not add nested focus targets");
  assert(managerSource.includes('<aside className="media-library"'), "The local library should render as a collapsible sidebar");
  assert(!managerSource.includes("libraryVisibilityInitialized"), "Background loading should never auto-open the local library");
  assert(!managerSource.includes("libraryItems[0] ?? null"), "Background loading should not silently select the first saved item");
  assert((managerSource.match(/setConfirmDeleteId\(""\)/g) ?? []).length >= 2, "Changing selection should disarm stale delete confirmation");
  assert(managerSource.includes('data-testid="media-variant-strip"'), "Multi-output image and video runs should expose a variant strip");
  assert(managerSource.includes('role="listbox" aria-label="Output variants"'), "The variant strip should expose listbox semantics");
  assert(managerSource.includes('data-testid="media-variant"'), "Every returned variant should remain directly selectable");
  assert(managerSource.includes('role="option"'), "Variant selectors should expose option semantics");
  assert(managerSource.includes("aria-selected={selectedVariantIndex === index}"), "The active variant should be exposed semantically");
  assert(managerSource.includes("onVariantKeyDown"), "Variants should support arrow, Home, and End keyboard selection");
  assert(managerSource.includes("setStageVariantIndex(0)"), "Changing the stage source should reset comparison to its first output");
  assert(managerSource.includes("latestResult.library_id"), "A generated run and its saved library record should share one variant-selection source");
  assert(managerSource.includes("media-stage-audio-list"), "Multi-output music runs should expose every returned audio output");

  const selectSource = readFileSync(resolve(process.cwd(), "src/components/ui.tsx"), "utf8");
  assert(selectSource.includes('role="listbox"'), "Shared select menus should use listbox semantics");
  assert(selectSource.includes('role="option"'), "Shared select items should expose option semantics");
  assert(selectSource.includes("aria-selected="), "Shared select options should expose selection");
  assert(selectSource.includes('event.key === "Escape"'), "Shared selects should close from Escape");
  assert(selectSource.includes('event.key === "Home"'), "Shared selects should support first-option keyboard navigation");
  assert(selectSource.includes('event.key === "End"'), "Shared selects should support last-option keyboard navigation");
  assert(selectSource.includes("closeAndFocusTrigger"), "Closing a shared select should restore trigger focus");

  const chatSource = readFileSync(resolve(process.cwd(), "src/components/ChatView.tsx"), "utf8");
  assert(chatSource.includes("<ComposerSurface>"), "Chat and Media Studio should share the same composer surface component");
  assert(chatSource.includes("popover"), "Chat should opt into the single-pill media controls");

  const styleSource = readFileSync(
    resolve(process.cwd(), "src/workspaces.css"),
    "utf8",
  );
  assert(styleSource.includes(".inline-media-parameter-controls"), "The shared inline media controls should retain compact parameter layout");
  assert(styleSource.includes("grid-auto-rows: max-content"), "Library cards should not stretch to fill an otherwise empty sidebar");
  assert(styleSource.includes(".inline-media-popover::before"), "The media settings surface should keep nested dropdown backdrop blur working");
  assert(styleSource.includes("border-bottom-right-radius: max(0px, calc(var(--card-radius) - 6px))"), "Resizable panels should use the shared inset theme-radius corner curve");
  assert(styleSource.includes("mask-composite: intersect"), "The resize curve tails should fade without dimming the corner");

  const apiSource = readFileSync(resolve(process.cwd(), "src/api.ts"), "utf8");
  assert(apiSource.includes("new URL(`${BASE}/media/library`)"), "The desktop API should list the media library");
  assert(apiSource.includes("/refresh"), "The desktop API should refresh pending or failed saves");
  assert(apiSource.includes("library_id?: string"), "Chat and studio media results should accept library IDs without breaking older responses");
} finally {
  await server.close();
}

export {};
