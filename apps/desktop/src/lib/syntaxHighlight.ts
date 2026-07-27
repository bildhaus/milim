import { createLowlight } from "lowlight";
import bash from "highlight.js/lib/languages/bash";
import css from "highlight.js/lib/languages/css";
import diff from "highlight.js/lib/languages/diff";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import markdown from "highlight.js/lib/languages/markdown";
import python from "highlight.js/lib/languages/python";
import rust from "highlight.js/lib/languages/rust";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";

export type SyntaxNode = {
  type?: string;
  tagName?: string;
  properties?: Record<string, unknown>;
  children?: SyntaxNode[];
  value?: string;
};

export type SyntaxToken = {
  text: string;
  className?: string;
};

const plainTextLanguages = new Set(["text", "txt", "plain", "plaintext"]);
const lowlight = createLowlight({
  bash,
  css,
  diff,
  javascript,
  json,
  markdown,
  python,
  rust,
  typescript,
  xml,
  yaml,
});

lowlight.registerAlias({
  bash: ["sh", "shell", "zsh", "ps1", "powershell"],
  javascript: ["js", "jsx", "mjs", "cjs"],
  markdown: ["md", "mdx"],
  rust: ["rs"],
  typescript: ["ts", "tsx"],
  xml: ["html", "htm", "svg"],
  yaml: ["yml"],
});

function classNames(value: unknown): string[] {
  if (typeof value === "string") return value.split(/\s+/).filter(Boolean);
  if (Array.isArray(value)) return value.flatMap(classNames);
  return [];
}

export function highlightSyntax(language: string | null | undefined, source: string): SyntaxNode | null {
  const normalized = language?.toLowerCase();
  if (!normalized || plainTextLanguages.has(normalized) || !lowlight.registered(normalized)) return null;
  try {
    return lowlight.highlight(normalized, source, { prefix: "hljs-" }) as SyntaxNode;
  } catch {
    return null;
  }
}

export function highlightedCodeLines(source: string, language?: string | null): SyntaxToken[][] {
  const highlighted = highlightSyntax(language, source);
  if (!highlighted) return source.split(/\r?\n/).map((text) => text ? [{ text }] : []);
  const lines: SyntaxToken[][] = [[]];

  function visit(node: SyntaxNode, inheritedClasses: string[]) {
    const classes = node.type === "element"
      ? [...inheritedClasses, ...classNames(node.properties?.className)]
      : inheritedClasses;
    if (node.type === "text") {
      const parts = (node.value ?? "").split(/\r?\n/);
      parts.forEach((text, index) => {
        if (text) lines[lines.length - 1].push({
          text,
          className: classes.length ? classes.join(" ") : undefined,
        });
        if (index < parts.length - 1) lines.push([]);
      });
      return;
    }
    for (const child of node.children ?? []) visit(child, classes);
  }

  visit(highlighted, []);
  return lines;
}
