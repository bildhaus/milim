import { Children, isValidElement, memo, useMemo, type ComponentProps, type MouseEvent, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import rehypeRaw from "rehype-raw";
import rehypeSanitize from "rehype-sanitize";
import remarkGfm from "remark-gfm";
import { openExternalUrl, type ChatArtifact } from "../api";
import { extractArtifactsFromContent, isPreviewableArtifact } from "../lib/artifacts";
import { markPerfRender } from "../lib/perf";
import { highlightSyntax, type SyntaxNode } from "../lib/syntaxHighlight";
import { CodeBlock } from "./CodeBlock";

type MarkdownRehypePlugins = NonNullable<ComponentProps<typeof ReactMarkdown>["rehypePlugins"]>;
type MarkdownProps = {
  content: string;
  previewArtifacts?: ChatArtifact[];
  onOpenPreview?: (artifact: ChatArtifact) => void;
  highlight?: boolean;
  allowHtml?: boolean;
  previewArtifactsStreaming?: boolean;
  collapseArtifacts?: boolean;
};

type MarkdownRehypePlugin = MarkdownRehypePlugins[number];
type HastNode = SyntaxNode;

function codeBlockText(children: ReactNode): string {
  return Children.toArray(children)
    .map((child) => {
      if (typeof child === "string" || typeof child === "number") return String(child);
      if (!isValidElement(child)) return "";
      return codeBlockText((child.props as { children?: ReactNode }).children);
    })
    .join("");
}

function normalizedCodeText(text: string): string {
  return text.replace(/\s+$/g, "");
}

function previewArtifactForCodeText(text: string, artifacts: ChatArtifact[]): ChatArtifact | undefined {
  return artifacts.find((artifact) => normalizedCodeText(artifact.content) === text);
}

function classNames(value: unknown): string[] {
  if (typeof value === "string") return value.split(/\s+/).filter(Boolean);
  if (Array.isArray(value)) return value.flatMap(classNames);
  return [];
}

function textContent(node: HastNode | undefined): string {
  if (!node) return "";
  if (node.type === "text") return node.value ?? "";
  return (node.children ?? []).map(textContent).join("");
}

function languageFromCode(node: HastNode): string | null {
  for (const className of classNames(node.properties?.className)) {
    const language = className.match(/^(?:language|lang)-(.+)$/)?.[1]?.toLowerCase();
    if (language) return language;
  }
  return null;
}

function selectedHighlightPlugin() {
  return (tree: HastNode) => {
    visit(tree);
  };

  function visit(node: HastNode): void {
    if (node.type === "element" && node.tagName === "pre") highlightPre(node);
    for (const child of node.children ?? []) visit(child);
  }

  function highlightPre(pre: HastNode): void {
    const code = pre.children?.find((child) => child.type === "element" && child.tagName === "code");
    if (!code) return;
    const language = languageFromCode(code);
    const highlighted = highlightSyntax(language, textContent(code));
    if (!highlighted) return;
    code.children = highlighted.children;
    code.properties = {
      ...code.properties,
      className: Array.from(new Set(["hljs", ...classNames(code.properties?.className)])),
    };
  }
}

const highlightRehypePlugins = [selectedHighlightPlugin as MarkdownRehypePlugin] satisfies MarkdownRehypePlugins;
const safeHtmlRehypePlugins = [rehypeRaw, rehypeSanitize] satisfies MarkdownRehypePlugins;
const safeHtmlHighlightRehypePlugins = [
  rehypeRaw,
  rehypeSanitize,
  selectedHighlightPlugin as MarkdownRehypePlugin,
] satisfies MarkdownRehypePlugins;

export function parseMarkdownIntoBlocks(content: string): string[] {
  const lines = content.match(/[^\n]*(?:\n|$)/g)?.filter(Boolean) ?? [];
  const blocks: string[] = [];
  let current: string[] = [];
  let inFence = false;

  const push = () => {
    const block = current.join("").trimEnd();
    if (block.trim()) blocks.push(block);
    current = [];
  };

  for (const line of lines) {
    current.push(line);
    if (/^\s*(```|~~~)/.test(line)) inFence = !inFence;
    if (!inFence && line.trim() === "") push();
  }
  push();
  return blocks;
}

export function isHttpHref(href: string | undefined): href is string {
  return Boolean(href && /^https?:\/\//i.test(href));
}

function openMarkdownLink(event: MouseEvent<HTMLAnchorElement>, href: string | undefined): void {
  if (!isHttpHref(href)) return;
  event.preventDefault();
  void openExternalUrl(href).catch((error) => console.warn("failed to open link", error));
}

function MarkdownBody({
  content,
  previewArtifacts,
  onOpenPreview,
  highlight = true,
  allowHtml = false,
  previewArtifactsStreaming = false,
  collapseArtifacts = true,
}: MarkdownProps) {
  const effectivePreviewArtifacts = useMemo(
    () =>
      collapseArtifacts
        ? previewArtifacts?.length
          ? previewArtifacts
          : extractArtifactsFromContent(content).filter(isPreviewableArtifact)
        : [],
    [collapseArtifacts, content, previewArtifacts],
  );

  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      rehypePlugins={
        allowHtml
          ? highlight
            ? safeHtmlHighlightRehypePlugins
            : safeHtmlRehypePlugins
          : highlight
            ? highlightRehypePlugins
            : undefined
      }
      components={{
        pre: ({ children }) => {
          const text = normalizedCodeText(codeBlockText(children));
          const previewArtifact = previewArtifactForCodeText(text, effectivePreviewArtifacts);
          if (!previewArtifact && !text.trim()) return null;
          return (
            <CodeBlock previewArtifact={previewArtifact} previewStreaming={Boolean(previewArtifact && previewArtifactsStreaming)} onOpenPreview={onOpenPreview}>
              {children}
            </CodeBlock>
          );
        },
        a: ({ children, href }) => (
          <a href={href} target="_blank" rel="noreferrer" onClick={(event) => openMarkdownLink(event, href)}>
            {children}
          </a>
        ),
      }}
    >
      {content}
    </ReactMarkdown>
  );
}

/** Render chat text as GitHub-flavored markdown with syntax-highlighted
 *  code blocks. Memoized so re-renders during streaming stay cheap. */
export const Markdown = memo(function Markdown(props: MarkdownProps) {
  markPerfRender("Markdown");
  return (
    <div className="md">
      <MarkdownBody {...props} />
    </div>
  );
});

const MemoizedMarkdownBlock = memo(function MemoizedMarkdownBlock(props: MarkdownProps) {
  markPerfRender("MarkdownBlock");
  return <MarkdownBody {...props} />;
});

export const MemoizedMarkdown = memo(function MemoizedMarkdown(props: MarkdownProps) {
  markPerfRender("MemoizedMarkdown");
  const blocks = useMemo(() => parseMarkdownIntoBlocks(props.content), [props.content]);
  return (
    <div className="md">
      {blocks.map((block, index) => (
        <MemoizedMarkdownBlock {...props} content={block} key={index} />
      ))}
    </div>
  );
});
