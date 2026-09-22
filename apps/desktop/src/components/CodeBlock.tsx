import { Children, isValidElement, useMemo, useRef, useState, type ReactNode } from "react";
import type { ChatArtifact } from "../api";
import { formatBytes, isPreviewableArtifact } from "../lib/artifacts";
import { Code, Copy, Eye } from "./icons";

function codeText(children: ReactNode): string {
  return Children.toArray(children)
    .map((child) => {
      if (typeof child === "string" || typeof child === "number") return String(child);
      if (!isValidElement(child)) return "";
      return codeText((child.props as { children?: ReactNode }).children);
    })
    .join("");
}

/** A fenced code block with a copy button. `children` is react-markdown's
 *  highlighted `<code>` element when the markdown renderer enabled highlighting. */
export function CodeBlock({
  children,
  previewArtifact,
  previewStreaming = false,
  onOpenPreview,
}: {
  children?: ReactNode;
  previewArtifact?: ChatArtifact;
  previewStreaming?: boolean;
  onOpenPreview?: (artifact: ChatArtifact) => void;
}) {
  const ref = useRef<HTMLPreElement>(null);
  const [copied, setCopied] = useState(false);
  const sourceText = useMemo(() => previewArtifact?.content ?? codeText(children), [children, previewArtifact?.content]);
  const collapsedPreview = Boolean(previewArtifact && onOpenPreview);

  const copy = async () => {
    const text = ref.current?.innerText ?? sourceText;
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      /* clipboard unavailable */
    }
  };

  if (collapsedPreview && previewArtifact) {
    const title = previewArtifact.filename ?? previewArtifact.title;
    const previewable = isPreviewableArtifact(previewArtifact);
    const openLabel = previewable ? "Open preview" : "Open code";
    const meta = [previewArtifact.language?.toUpperCase(), previewArtifact.mime, previewStreaming ? "Streaming..." : formatBytes(previewArtifact.size)]
      .filter(Boolean)
      .join(" - ");
    return (
      <div className="code-block code-block-collapsed" data-testid="code-artifact-collapsed">
        <div className="code-artifact-main">
          <span className="code-artifact-icon" aria-hidden="true">
            <Code size={14} />
          </span>
          <span className="code-artifact-copy">
            <span className="code-artifact-title" title={title}>
              {title}
            </span>
            <span className="code-artifact-meta">{meta}</span>
          </span>
        </div>
        <div className="code-artifact-actions">
          <button
            className="code-action code-preview"
            data-testid="code-preview"
            onClick={() => onOpenPreview?.(previewArtifact)}
            title={openLabel}
            aria-label={openLabel}
          >
            {previewable ? <Eye size={13} /> : <Code size={13} />}
          </button>
          <button
            className="code-action code-copy"
            data-testid="code-copy"
            onClick={copy}
            title={copied ? "Copied" : "Copy code"}
            aria-label={copied ? "Copied" : "Copy code"}
          >
            <Copy size={13} />
            <span>{copied ? "Copied" : "Copy"}</span>
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="code-block">
      <div className="code-block-actions">
        {previewArtifact && onOpenPreview && (
          <button
            className="code-action code-preview"
            data-testid="code-preview"
            onClick={() => onOpenPreview(previewArtifact)}
            title="Open preview"
            aria-label="Open preview"
          >
            <Eye size={13} />
          </button>
        )}
        <button
          className="code-action code-copy"
          data-testid="code-copy"
          onClick={copy}
          title={copied ? "Copied" : "Copy code"}
          aria-label={copied ? "Copied" : "Copy code"}
        >
          <Copy size={13} />
          <span>{copied ? "Copied" : "Copy"}</span>
        </button>
      </div>
      <pre ref={ref}>{children}</pre>
    </div>
  );
}
