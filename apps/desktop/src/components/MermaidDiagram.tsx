import { useEffect, useId, useState } from "react";
import { useTheme } from "../theme/store";
import { Copy, Download, Image as ImageIcon } from "./icons";
import { CodeBlock } from "./CodeBlock";

const MAX_RASTER_DIMENSION = 4096;
const DEFAULT_SVG_WIDTH = 800;
const DEFAULT_SVG_HEIGHT = 450;
let renderSequence = 0;

export function standaloneMermaidSvg(svg: string): string {
  const trimmed = svg.trim();
  const withNamespace = /<svg\b[^>]*\sxmlns=/.test(trimmed)
    ? trimmed
    : trimmed.replace(/<svg\b/, '<svg xmlns="http://www.w3.org/2000/svg"');
  return `<?xml version="1.0" encoding="UTF-8"?>\n${withNamespace}`;
}

export function mermaidSvgDimensions(svg: string): { width: number; height: number } {
  const opening = svg.match(/<svg\b[^>]*>/i)?.[0] ?? "";
  const viewBox = opening.match(/\bviewBox\s*=\s*["']\s*([^"']+)["']/i)?.[1]
    ?.trim()
    .split(/[\s,]+/)
    .map(Number);
  if (viewBox?.length === 4 && viewBox.every(Number.isFinite) && viewBox[2] > 0 && viewBox[3] > 0) {
    return { width: viewBox[2], height: viewBox[3] };
  }
  const width = Number.parseFloat(opening.match(/\bwidth\s*=\s*["']([^"'%]+)["']/i)?.[1] ?? "");
  const height = Number.parseFloat(opening.match(/\bheight\s*=\s*["']([^"'%]+)["']/i)?.[1] ?? "");
  return {
    width: Number.isFinite(width) && width > 0 ? width : DEFAULT_SVG_WIDTH,
    height: Number.isFinite(height) && height > 0 ? height : DEFAULT_SVG_HEIGHT,
  };
}

export function boundedRasterDimensions(
  width: number,
  height: number,
  scale = 2,
  maxDimension = MAX_RASTER_DIMENSION,
): { width: number; height: number } {
  const safeWidth = Number.isFinite(width) && width > 0 ? width : DEFAULT_SVG_WIDTH;
  const safeHeight = Number.isFinite(height) && height > 0 ? height : DEFAULT_SVG_HEIGHT;
  const factor = Math.min(scale, maxDimension / safeWidth, maxDimension / safeHeight);
  return {
    width: Math.max(1, Math.round(safeWidth * factor)),
    height: Math.max(1, Math.round(safeHeight * factor)),
  };
}

function escapeSvgAttribute(value: string): string {
  return value.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function rasterSvg(svg: string, background: string, width: number, height: number): string {
  return standaloneMermaidSvg(svg).replace(/<svg\b([^>]*)>/i, (_match, attributes: string) => {
    const sized = attributes
      .replace(/\swidth\s*=\s*["'][^"']*["']/i, "")
      .replace(/\sheight\s*=\s*["'][^"']*["']/i, "");
    return `<svg${sized} width="${width}" height="${height}"><rect width="100%" height="100%" fill="${escapeSvgAttribute(background)}"/>`;
  });
}

export async function mermaidPngBlob(svg: string, background: string): Promise<Blob> {
  const source = mermaidSvgDimensions(svg);
  const size = boundedRasterDimensions(source.width, source.height);
  const blob = new Blob([rasterSvg(svg, background, size.width, size.height)], { type: "image/svg+xml;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  try {
    const image = new window.Image();
    image.decoding = "async";
    await new Promise<void>((resolve, reject) => {
      image.onload = () => resolve();
      image.onerror = () => reject(new Error("The diagram could not be converted to an image."));
      image.src = url;
    });
    const canvas = document.createElement("canvas");
    canvas.width = size.width;
    canvas.height = size.height;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Image export is unavailable.");
    context.fillStyle = background;
    context.fillRect(0, 0, size.width, size.height);
    context.drawImage(image, 0, 0, size.width, size.height);
    return await new Promise<Blob>((resolve, reject) =>
      canvas.toBlob((png) => png ? resolve(png) : reject(new Error("PNG export failed.")), "image/png"),
    );
  } finally {
    URL.revokeObjectURL(url);
  }
}

function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  document.body.append(link);
  link.click();
  link.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 0);
}

function readableError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return message.trim().slice(0, 1200) || "Mermaid could not render this diagram.";
}

export function MermaidDiagram({ source }: { source: string }) {
  const componentId = useId().replace(/[^a-z0-9_-]/gi, "");
  const theme = useTheme((state) => state.theme);
  const [view, setView] = useState<"diagram" | "code">("diagram");
  const [svg, setSvg] = useState("");
  const [status, setStatus] = useState<"rendering" | "ready" | "error">("rendering");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");

  useEffect(() => setView("diagram"), [source]);
  useEffect(() => {
    let active = true;
    setSvg("");
    setError("");
    setStatus("rendering");
    void import("mermaid")
      .then(async ({ default: mermaid }) => {
        mermaid.initialize({
          startOnLoad: false,
          securityLevel: "strict",
          suppressErrorRendering: true,
          maxTextSize: 50_000,
          maxEdges: 500,
          htmlLabels: false,
          flowchart: { htmlLabels: false },
          theme: "base",
          darkMode: theme.isDark,
          fontFamily: theme.typography.fontFamily,
          themeVariables: {
            background: theme.colors.cardBg,
            primaryColor: theme.colors.bgTertiary,
            primaryTextColor: theme.colors.primaryText,
            primaryBorderColor: theme.colors.accent,
            lineColor: theme.colors.secondaryText,
            secondaryColor: theme.colors.bgSecondary,
            tertiaryColor: theme.colors.bgPrimary,
          },
        });
        const result = await mermaid.render(`milim-mermaid-${componentId}-${++renderSequence}`, source);
        if (!active) return;
        setSvg(result.svg);
        setStatus("ready");
      })
      .catch((reason) => {
        if (!active) return;
        setError(readableError(reason));
        setStatus("error");
        setView("code");
      });
    return () => {
      active = false;
    };
  }, [componentId, source, theme]);

  async function runAction(action: () => Promise<void> | void, success: string): Promise<void> {
    setNotice("");
    try {
      await action();
      setNotice(success);
    } catch (reason) {
      setNotice(readableError(reason));
    }
  }

  const canCopyImage = typeof navigator !== "undefined"
    && typeof navigator.clipboard?.write === "function"
    && typeof ClipboardItem !== "undefined";
  const diagramTabId = `${componentId}-diagram-tab`;
  const codeTabId = `${componentId}-code-tab`;
  const diagramPanelId = `${componentId}-diagram-panel`;
  const codePanelId = `${componentId}-code-panel`;

  return (
    <section className="mermaid-card" data-testid="mermaid-diagram" aria-label="Mermaid diagram">
      <header className="mermaid-header">
        <div className="mermaid-tabs" role="tablist" aria-label="Mermaid view">
          <button
            type="button"
            role="tab"
            id={diagramTabId}
            aria-controls={diagramPanelId}
            aria-selected={view === "diagram"}
            disabled={status === "error"}
            onClick={() => setView("diagram")}
          >
            Diagram
          </button>
          <button
            type="button"
            role="tab"
            id={codeTabId}
            aria-controls={codePanelId}
            aria-selected={view === "code"}
            onClick={() => setView("code")}
          >
            Code
          </button>
        </div>
        <div className="mermaid-actions">
          <button
            type="button"
            title="Copy Mermaid code"
            onClick={() => void runAction(async () => {
              if (!navigator.clipboard?.writeText) throw new Error("Text clipboard is unavailable.");
              await navigator.clipboard.writeText(source);
            }, "Copied Mermaid code")}
          >
            <Copy size={13} />
            <span>Code</span>
          </button>
          <button
            type="button"
            title={canCopyImage ? "Copy diagram image" : "Image clipboard is unavailable"}
            disabled={!svg || !canCopyImage}
            onClick={() => void runAction(async () => {
              const png = await mermaidPngBlob(svg, theme.colors.cardBg);
              await navigator.clipboard.write([new ClipboardItem({ "image/png": png })]);
            }, "Copied diagram image")}
          >
            <ImageIcon size={13} />
            <span>Image</span>
          </button>
          <button
            type="button"
            title="Download SVG"
            disabled={!svg}
            onClick={() => void runAction(() => downloadBlob(
              new Blob([standaloneMermaidSvg(svg)], { type: "image/svg+xml;charset=utf-8" }),
              "mermaid-diagram.svg",
            ), "Downloaded SVG")}
          >
            <Download size={13} />
            <span>SVG</span>
          </button>
          <button
            type="button"
            title="Download PNG"
            disabled={!svg}
            onClick={() => void runAction(async () => downloadBlob(
              await mermaidPngBlob(svg, theme.colors.cardBg),
              "mermaid-diagram.png",
            ), "Downloaded PNG")}
          >
            <Download size={13} />
            <span>PNG</span>
          </button>
        </div>
      </header>
      {error && <div className="mermaid-error" role="alert">{error}</div>}
      {view === "code" ? (
        <div className="mermaid-source" role="tabpanel" id={codePanelId} aria-labelledby={codeTabId}>
          <CodeBlock><code className="language-mermaid">{source}</code></CodeBlock>
        </div>
      ) : (
        <div className="mermaid-canvas" role="tabpanel" id={diagramPanelId} aria-labelledby={diagramTabId}>
          {status === "rendering" ? (
            <div className="mermaid-status" role="status">Rendering diagram...</div>
          ) : (
            <div className="mermaid-svg" dangerouslySetInnerHTML={{ __html: svg }} />
          )}
        </div>
      )}
      {notice && <div className="mermaid-notice" aria-live="polite">{notice}</div>}
    </section>
  );
}
