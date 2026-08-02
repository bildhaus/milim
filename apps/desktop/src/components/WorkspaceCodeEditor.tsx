import { useEffect, useRef } from "react";
import { indentWithTab } from "@codemirror/commands";
import { StreamLanguage, HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { c, cpp, csharp, java, kotlin } from "@codemirror/legacy-modes/mode/clike";
import { css } from "@codemirror/legacy-modes/mode/css";
import { javascript, json, typescript } from "@codemirror/legacy-modes/mode/javascript";
import { python } from "@codemirror/legacy-modes/mode/python";
import { rust } from "@codemirror/legacy-modes/mode/rust";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import { toml } from "@codemirror/legacy-modes/mode/toml";
import { html, xml } from "@codemirror/legacy-modes/mode/xml";
import { yaml } from "@codemirror/legacy-modes/mode/yaml";
import { EditorState, type Extension } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { tags } from "@lezer/highlight";
import { basicSetup } from "codemirror";

function languageFor(filename: string): Extension[] {
  const extension = filename.split(".").pop()?.toLowerCase();
  const mode = extension && {
    c, cc: cpp, cpp, cxx: cpp, cs: csharp, css, h: c, hpp: cpp,
    html, htm: html, java, js: javascript, jsx: javascript, json,
    kt: kotlin, kts: kotlin, py: python, rs: rust, sh: shell,
    bash: shell, toml, ts: typescript, tsx: typescript, xml, yaml, yml: yaml,
  }[extension];
  return mode ? [StreamLanguage.define(mode)] : [];
}

const editorTheme = EditorView.theme({
  "&": { height: "100%", backgroundColor: "transparent", color: "var(--workspace-code-fg)" },
  ".cm-scroller": { fontFamily: "var(--mono)", lineHeight: "1.6", overflow: "auto" },
  ".cm-content": { caretColor: "var(--workspace-code-caret)", padding: "10px 0 32px" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--workspace-code-caret)" },
  ".cm-gutters": { backgroundColor: "var(--workspace-code-gutter)", color: "var(--workspace-code-muted)", border: "0" },
  ".cm-activeLine, .cm-activeLineGutter": { backgroundColor: "var(--workspace-code-active)" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground, ::selection": { backgroundColor: "var(--workspace-code-selection) !important" },
  ".cm-panels": { backgroundColor: "var(--workspace-code-panel)", color: "inherit" },
  ".cm-panels.cm-panels-top": { borderBottom: "1px solid var(--border-primary)" },
  ".cm-searchMatch": { backgroundColor: "var(--workspace-code-search)" },
  "&.cm-focused": { outline: "none" },
});

const highlightTheme = HighlightStyle.define([
  { tag: [tags.keyword, tags.modifier], color: "var(--workspace-code-keyword)" },
  { tag: [tags.string, tags.regexp], color: "var(--workspace-code-string)" },
  { tag: [tags.number, tags.bool, tags.null], color: "var(--workspace-code-number)" },
  { tag: [tags.comment, tags.meta], color: "var(--workspace-code-comment)" },
  { tag: [tags.function(tags.variableName), tags.labelName], color: "var(--workspace-code-function)" },
  { tag: [tags.typeName, tags.className], color: "var(--workspace-code-type)" },
  { tag: [tags.tagName, tags.attributeName], color: "var(--workspace-code-tag)" },
]);

export default function WorkspaceCodeEditor({
  filename,
  source,
  onChange,
  onSave,
}: {
  filename: string;
  source: string;
  onChange: (source: string) => void;
  onSave: () => void;
}) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const viewRef = useRef<EditorView | null>(null);
  const onChangeRef = useRef(onChange);
  const onSaveRef = useRef(onSave);
  onChangeRef.current = onChange;
  onSaveRef.current = onSave;

  useEffect(() => {
    if (!hostRef.current) return;
    const view = new EditorView({
      parent: hostRef.current,
      state: EditorState.create({
        doc: source,
        extensions: [
          basicSetup,
          EditorState.lineSeparator.of(source.includes("\r\n") ? "\r\n" : "\n"),
          keymap.of([{ key: "Mod-s", preventDefault: true, run: () => { onSaveRef.current(); return true; } }, indentWithTab]),
          EditorView.contentAttributes.of({ "aria-label": `${filename} editor` }),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) onChangeRef.current(update.state.doc.toString());
          }),
          editorTheme,
          syntaxHighlighting(highlightTheme),
          ...languageFor(filename),
        ],
      }),
    });
    viewRef.current = view;
    return () => {
      view.destroy();
      viewRef.current = null;
    };
  }, [filename]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view || view.state.doc.toString() === source) return;
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: source } });
  }, [source]);

  return <div ref={hostRef} className="workspace-code-editor" data-testid="workspace-code-editor" />;
}
