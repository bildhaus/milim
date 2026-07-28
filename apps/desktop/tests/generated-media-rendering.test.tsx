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

  const manager = renderToStaticMarkup(
    createElement(MediaManager, { onClose: () => {} }),
  );
  assert(manager.includes('data-testid="media-generator"'), "The standalone media manager should remain reachable");
  assert(manager.includes('data-testid="inline-media-generator"'), "The media manager should expose the chat media controls");
  assert(manager.includes('data-testid="media-stage"'), "The studio should expose a dedicated output stage");
  assert(manager.includes('aria-controls="media-library-sidebar"'), "The studio should expose a local-library sidebar toggle");
  assert(manager.includes('data-testid="inline-media-advanced-input"'), "Raw media input should reuse the chat composer disclosure");
  assert(manager.includes("media-composer-dock"), "The standalone generator should use a full-width composer dock");
  assert(manager.includes("dock-surface media-composer-surface"), "The prompt should use the shared chat composer surface");
  assert(manager.includes("composer-input media-composer-prompt"), "The media prompt should use the chat composer input structure");
  assert(manager.includes("control-bar media-control-bar"), "Model selection should use the chat composer control bar");
  assert(manager.includes('data-testid="media-model-picker-trigger"'), "The composer model chip should be the model picker entry point");
  assert(manager.includes('data-testid="inline-media-generator"'), "Generation parameters should reuse the chat media controls");
  assert(manager.includes("send-btn media-composer-send-btn"), "Generation should reuse the chat send-button treatment");
  assert(manager.includes('aria-label="Generate image"'), "The icon-only generation action should retain an accessible label");
  assert(manager.includes('data-testid="media-studio-resize-handle"'), "The studio should expose a keyboard-operable resize handle");
  assert(manager.includes("media-sheet-resize-glyph"), "The resize handle should use an inset three-line glyph");
  assert(manager.includes("Ctrl/Cmd + Enter"), "The generation shortcut should be shown outside the primary action label");
  assert(manager.includes("Quick generations here. Iteration stays in chat."), "The studio should explain its role without competing with chat");
  assert(manager.includes("Prompt sent unchanged"), "The privacy summary should remain concise in the generator rail");
  assert(manager.includes("Your next output will appear here"), "The initial preview should explain where results appear");
  assert(!manager.includes('data-testid="inline-media-settings-summary"'), "Media Studio should retain inline media controls");

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
  assert(chatMediaControls.includes('data-testid="inline-media-advanced-input"'), "Popover media controls should retain Advanced input");
  assert(chatMediaControls.includes('data-testid="inline-media-error"'), "Popover media controls should retain the detailed error");

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

  const managerSource = readFileSync(resolve(process.cwd(), "src/components/MediaManager.tsx"), "utf8");
  assert(managerSource.includes("event.ctrlKey || event.metaKey"), "The studio should generate with Ctrl/Cmd+Enter");
  assert(managerSource.includes('aria-live="polite"'), "Generation state should be announced to assistive technology");
  assert(managerSource.includes("confirmDeleteId"), "Permanent deletion should require a second confirmation action");
  assert(managerSource.includes("reuseLibraryItem"), "Saved prompt and settings should be reusable");
  assert(managerSource.includes("<ModelPicker"), "Media Studio should reuse the chat composer model picker");
  assert(managerSource.includes("<InlineMediaControls"), "Media Studio should reuse the chat media control row");
  assert(managerSource.includes('["image", "video", "music"] as MediaKind[]'), "The shared model picker should discover every media kind");
  assert(managerSource.includes("route.kinds.includes(kind)"), "The selected media type should filter the model picker");
  assert(managerSource.includes("bottom: window.innerHeight - rect.top + gap"), "Short upward-opening model pickers should stay anchored to their trigger");
  assert(!managerSource.includes('testId="media-model-select"'), "The model picker should not duplicate search and selection controls");
  assert(!managerSource.includes('className="field media-model-field"'), "Generation settings should not repeat the model picker");
  assert(managerSource.includes("favoriteIds={favoriteModelIds}"), "The shared picker should retain provider-scoped media favorites");
  assert(managerSource.includes("setMediaStudioSize"), "The resized studio dimensions should be persisted");
  assert(managerSource.includes("Saving locally..."), "The preview should distinguish local saving from generation");
  assert(managerSource.includes("This run failed"), "The preview should present a specific failed state");
  assert(managerSource.includes("Loading local library..."), "Initial library loading should not leave a blank grid");
  assert(managerSource.includes('stageStatus !== "ready"'), "Ready outputs should not repeat their state in the stage header");
  assert(managerSource.includes('className="media-library-count"'), "The library total should render as a distinct count badge");
  assert(managerSource.includes('item.save_state !== "ready"'), "Ready library cards should not repeat their state over the thumbnail");
  assert(managerSource.includes('className="media-library-card-meta"'), "Library cards should separate prompt and source metadata");
  assert(managerSource.includes("showLibraryFilters"), "Empty libraries should hide inactive filter chrome");
  assert(managerSource.includes("aria-current="), "The selected library card should be exposed semantically");
  assert(managerSource.includes('<aside className="media-library"'), "The local library should render as a collapsible sidebar");
  assert((managerSource.match(/setConfirmDeleteId\(""\)/g) ?? []).length >= 2, "Changing selection should disarm stale delete confirmation");

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

  const apiSource = readFileSync(resolve(process.cwd(), "src/api.ts"), "utf8");
  assert(apiSource.includes("new URL(`${BASE}/media/library`)"), "The desktop API should list the media library");
  assert(apiSource.includes("/refresh"), "The desktop API should refresh pending or failed saves");
  assert(apiSource.includes("library_id?: string"), "Chat and studio media results should accept library IDs without breaking older responses");
} finally {
  await server.close();
}

export {};
