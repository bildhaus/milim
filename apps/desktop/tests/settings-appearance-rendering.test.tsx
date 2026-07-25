import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import type {
  AvatarStyle,
  BackgroundFit,
  BackgroundTreatment,
  ChatLayoutStyle,
  CodeBlockTheme,
  MessageWidth,
} from "../src/ui/store.js";
import type { Project } from "../src/sessions/store.js";

type ChoiceProps<T extends string> = {
  value: T;
  onChange: (value: T) => void;
};

type BackgroundProps = {
  backgroundImage?: string;
  fit: BackgroundFit;
  treatment: BackgroundTreatment;
  onFitChange: (value: BackgroundFit) => void;
  onTreatmentChange: (value: BackgroundTreatment) => void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

function count(haystack: string, needle: string): number {
  return haystack.split(needle).length - 1;
}

const server = await createServer({
  root: process.cwd(),
  appType: "custom",
  logLevel: "silent",
  server: { middlewareMode: true },
});

try {
  const {
    AppearanceAvatarChoices,
    AppearanceBackgroundImageChoices,
    AppearanceChatLayoutChoices,
    AppearanceCodeBlockThemeChoices,
    AppearanceMessageWidthChoices,
  } = (await server.ssrLoadModule("/src/settings/AppearancePreviewChoices.tsx")) as {
    AppearanceAvatarChoices: ComponentType<ChoiceProps<AvatarStyle>>;
    AppearanceBackgroundImageChoices: ComponentType<BackgroundProps>;
    AppearanceChatLayoutChoices: ComponentType<ChoiceProps<ChatLayoutStyle>>;
    AppearanceCodeBlockThemeChoices: ComponentType<ChoiceProps<CodeBlockTheme>>;
    AppearanceMessageWidthChoices: ComponentType<ChoiceProps<MessageWidth>>;
  };
  const { ProjectCustomizationDialog } = (await server.ssrLoadModule("/src/components/Sidebar.tsx")) as {
    ProjectCustomizationDialog: ComponentType<{
      project: Project;
      onClose: () => void;
      onSave: () => void;
    }>;
  };

  const markup = [
    renderToStaticMarkup(createElement(AppearanceChatLayoutChoices, { value: "bubbles", onChange: () => {} })),
    renderToStaticMarkup(createElement(AppearanceMessageWidthChoices, { value: "wide", onChange: () => {} })),
    renderToStaticMarkup(createElement(AppearanceAvatarChoices, { value: "role", onChange: () => {} })),
    renderToStaticMarkup(createElement(AppearanceCodeBlockThemeChoices, { value: "terminal", onChange: () => {} })),
    renderToStaticMarkup(createElement(AppearanceBackgroundImageChoices, {
      backgroundImage: "linear-gradient(135deg, #101114, #30333a)",
      fit: "cover",
      treatment: "blur",
      onFitChange: () => {},
      onTreatmentChange: () => {},
    })),
  ].join("\n");

  equal(count(markup, 'role="radiogroup"'), 6, "Appearance controls should render one radiogroup per setting dimension");
  equal(count(markup, 'aria-checked="true"'), 6, "Each radiogroup should expose the selected option");
  assert(markup.includes('aria-checked="false"'), "Unselected radio options should remain accessible");
  assert(markup.includes("appearance-choice-card active"), "Selected options should render an active state");
  assert(markup.includes('role="slider"'), "Message width should render as a slider control");
  assert(markup.includes("appearance-width-stop active"), "Selected message width stop should render an active state");

  assert(markup.includes('data-testid="appearance-message-width-slider"'), "Message width slider test id should stay stable");
  assert(markup.includes('data-testid="appearance-message-width-wide"'), "Message width option test ids should stay stable");
  assert(markup.includes('data-testid="appearance-code-theme-terminal"'), "Code theme test id should stay stable");
  assert(markup.includes('data-testid="appearance-background-fit-cover"'), "Background fit test id should stay stable");
  assert(markup.includes('data-testid="appearance-background-treatment-blur"'), "Background treatment test id should stay stable");

  assert(markup.includes("appearance-chat-preview bubbles"), "Chat layout preview classes should render");
  assert(markup.includes("appearance-width-slider-preview wide"), "Message width slider preview classes should render");
  assert(markup.includes("appearance-avatar-preview role"), "Avatar preview classes should render");
  assert(markup.includes("Avatars"), "Appearance choices should offer model avatars instead of initials");
  assert(markup.includes('data-avatar-seed="appearance-preview-model"'), "Avatar preview should use the shared deterministic renderer");
  assert(markup.includes("appearance-code-preview terminal"), "Code theme preview classes should render");
  assert(markup.includes("appearance-background-option active"), "Selected background options should render an active state");
  assert(markup.includes('data-testid="appearance-background-preview"'), "Background controls should render the selected image thumbnail");
  assert(markup.includes("appearance-background-thumbnail bg-fit-cover bg-treatment-blur"), "Background thumbnail should reflect selected fit and treatment");
  assert(markup.includes("appearance-background-glyph fit-cover"), "Background fit options should render semantic glyphs");
  assert(markup.includes("appearance-background-glyph treatment-blur"), "Background treatment options should render semantic glyphs");
  assert(markup.includes("appearance-background-selection-detail"), "Background controls should render one selected-option description per group");
  assert(!markup.includes("appearance-background-live"), "Background controls should not render a live image preview");
  assert(!markup.includes("appearance-bg-thumb"), "Background controls should not render thumbnail previews");

  const noBackgroundMarkup = renderToStaticMarkup(createElement(AppearanceBackgroundImageChoices, {
    fit: "cover",
    treatment: "clear",
    onFitChange: () => {},
    onTreatmentChange: () => {},
  }));
  equal(noBackgroundMarkup, "", "Background controls should not render when the active theme has no image");

  const projectMarkup = renderToStaticMarkup(createElement(ProjectCustomizationDialog, {
    project: {
      id: "project:C:\\workspace",
      name: "Workspace",
      folder: "C:\\workspace",
      icon: "terminal",
      color: "#22aa88",
      createdAt: 1,
      updatedAt: 1,
    },
    onClose: () => {},
    onSave: () => {},
  }));
  assert(projectMarkup.includes('role="dialog"'), "Project customization should render as an accessible dialog");
  assert(projectMarkup.includes('aria-label="Project name"'), "Project customization should label its name field");
  equal(count(projectMarkup, 'role="radio"'), 12, "Project customization should offer the curated icon set");
  equal(count(projectMarkup, 'aria-checked="true"'), 1, "Project customization should expose the selected icon");
  assert(projectMarkup.includes("Use default"), "Project customization should provide a default-color reset");
  assert(projectMarkup.includes('aria-label="Project preview"'), "Project customization should render a live preview");
} finally {
  await server.close();
}

export {};
