import {
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
  type WheelEvent as ReactWheelEvent,
} from "react";
import { createPortal } from "react-dom";
import {
  loadAuthenticatedMedia,
  type MediaResultItem,
} from "../api";
import { ExternalLink, Image, Volume2, X } from "./icons";
import { SheetDialog } from "./SheetDialog";
import "./GeneratedMedia.css";

const MEDIA_ZOOM_MIN = 50;
const MEDIA_ZOOM_MAX = 300;
const MEDIA_ZOOM_STEP = 25;

function isDirectWebUrl(url: string): boolean {
  return /^https?:\/\//i.test(url);
}
export function GeneratedMedia({
  item,
  alt,
  onOpenExternal,
  onActivate,
  interactive = true,
  pressed,
}: {
  item?: MediaResultItem | null;
  alt: string;
  onOpenExternal?: (url: string) => void;
  onActivate?: () => void;
  interactive?: boolean;
  pressed?: boolean;
}) {
  const [previewOpen, setPreviewOpen] = useState(false);
  const [source, setSource] = useState(item?.requires_auth ? "" : item?.url ?? "");
  const [loadError, setLoadError] = useState(false);
  const [zoom, setZoom] = useState(100);
  const panStart = useRef<{ pointerId: number; x: number; y: number; left: number; top: number } | null>(null);
  const ignoreNextClick = useRef(false);

  function startPan(event: ReactPointerEvent<HTMLDivElement>) {
    if (zoom <= 100 || event.button !== 0 || (video && event.target instanceof Element && event.target.closest("video"))) return;
    panStart.current = {
      pointerId: event.pointerId,
      x: event.clientX,
      y: event.clientY,
      left: event.currentTarget.scrollLeft,
      top: event.currentTarget.scrollTop,
    };
    event.currentTarget.setPointerCapture(event.pointerId);
    event.currentTarget.classList.add("is-panning");
    event.preventDefault();
  }

  function movePan(event: ReactPointerEvent<HTMLDivElement>) {
    const start = panStart.current;
    if (!start || start.pointerId !== event.pointerId) return;
    if (Math.abs(event.clientX - start.x) > 3 || Math.abs(event.clientY - start.y) > 3) {
      ignoreNextClick.current = true;
    }
    event.currentTarget.scrollLeft = start.left - (event.clientX - start.x);
    event.currentTarget.scrollTop = start.top - (event.clientY - start.y);
  }

  function endPan(event: ReactPointerEvent<HTMLDivElement>) {
    if (panStart.current?.pointerId !== event.pointerId) return;
    panStart.current = null;
    event.currentTarget.classList.remove("is-panning");
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    if (ignoreNextClick.current) {
      window.setTimeout(() => {
        ignoreNextClick.current = false;
      }, 0);
    }
  }

  function zoomWithWheel(event: ReactWheelEvent<HTMLDivElement>) {
    event.preventDefault();
    const direction = event.deltaY < 0 ? 1 : -1;
    setZoom((value) =>
      Math.min(
        MEDIA_ZOOM_MAX,
        Math.max(MEDIA_ZOOM_MIN, value + direction * MEDIA_ZOOM_STEP),
      ),
    );
  }

  function closeFromBackdropClick(event: ReactMouseEvent<HTMLDivElement>) {
    if (ignoreNextClick.current) {
      ignoreNextClick.current = false;
      return;
    }

    const target = event.target;
    if (
      target === event.currentTarget ||
      (target instanceof HTMLElement && target.classList.contains("generated-media-canvas"))
    ) {
      setPreviewOpen(false);
      return;
    }
    if (!(target instanceof HTMLImageElement || target instanceof HTMLVideoElement)) return;

    const sourceWidth = target instanceof HTMLImageElement ? target.naturalWidth : target.videoWidth;
    const sourceHeight = target instanceof HTMLImageElement ? target.naturalHeight : target.videoHeight;
    if (!sourceWidth || !sourceHeight) return;
    const bounds = target.getBoundingClientRect();
    const scale = Math.min(bounds.width / sourceWidth, bounds.height / sourceHeight);
    const contentWidth = sourceWidth * scale;
    const contentHeight = sourceHeight * scale;
    const x = event.clientX - bounds.left;
    const y = event.clientY - bounds.top;
    if (
      x < (bounds.width - contentWidth) / 2 ||
      x > (bounds.width + contentWidth) / 2 ||
      y < (bounds.height - contentHeight) / 2 ||
      y > (bounds.height + contentHeight) / 2
    ) {
      setPreviewOpen(false);
    }
  }

  useEffect(() => {
    setLoadError(false);
    if (!item?.requires_auth) {
      setSource(item?.url ?? "");
      return;
    }
    const controller = new AbortController();
    let objectUrl = "";
    setSource("");
    loadAuthenticatedMedia(item.url, controller.signal)
      .then((blob) => {
        objectUrl = URL.createObjectURL(blob);
        setSource(objectUrl);
      })
      .catch((error) => {
        if (error instanceof DOMException && error.name === "AbortError") return;
        setLoadError(true);
      });
    return () => {
      controller.abort();
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [item?.url, item?.requires_auth]);

  if (!item?.url) {
    return <div className="generated-media-placeholder"><Image size={26} /></div>;
  }

  if (item.kind === "music") {
    if (!interactive) {
      return (
        <span className="generated-media-audio passive" data-testid="generated-media-music" role="img" aria-label={alt}>
          <Volume2 size={18} aria-hidden="true" />
          <span>{source ? "Audio" : loadError ? "Music unavailable" : "Loading music..."}</span>
        </span>
      );
    }
    return (
      <div className="generated-media-audio" data-testid="generated-media-music">
        <Volume2 size={18} aria-hidden="true" />
        {source ? (
          <audio controls preload="metadata" src={source} aria-label={alt} />
        ) : (
          <span>{loadError ? "Music unavailable" : "Loading music..."}</span>
        )}
      </div>
    );
  }

  const video = item.kind === "video";
  const canOpenExternally = isDirectWebUrl(item.url);
  if (!interactive) {
    return (
      <span
        className="generated-media-thumbnail passive"
        data-testid={`generated-media-${video ? "video" : "image"}`}
        role="img"
        aria-label={alt}
      >
        {source ? (
          video ? (
            <video src={source} muted preload="metadata" aria-hidden="true" />
          ) : (
            <img src={source} alt="" />
          )
        ) : (
          <span>{loadError ? "Media unavailable" : "Loading media..."}</span>
        )}
      </span>
    );
  }
  return (
    <>
      <button
        className="generated-media-thumbnail"
        data-testid={`generated-media-${video ? "video" : "image"}`}
        type="button"
        onClick={() => {
          if (onActivate) onActivate();
          else {
            setZoom(100);
            setPreviewOpen(true);
          }
        }}
        disabled={!source}
        aria-label={onActivate ? alt : `Preview ${alt}`}
        aria-pressed={onActivate && pressed !== undefined ? pressed : undefined}
      >
        {source ? (
          video ? (
            <video src={source} muted preload="metadata" aria-label={alt} />
          ) : (
            <img src={source} alt={alt} />
          )
        ) : (
          <span>{loadError ? "Media unavailable" : "Loading media..."}</span>
        )}
      </button>

      {previewOpen && source && createPortal(
        <SheetDialog
          title={alt}
          className="generated-media-dialog"
          overlayClassName="generated-media-overlay"
          testId="generated-media-preview"
          onClose={() => setPreviewOpen(false)}
        >
          <div className="generated-media-toolbar">
            <span>{alt}</span>
            <div>
              <div className="topbar-zoom-chip generated-media-zoom" role="group" aria-label="Media zoom controls">
                <span className="topbar-zoom-value" aria-live="polite">{zoom}%</span>
                <button
                  className="topbar-zoom-btn"
                  data-testid="media-zoom-decrease"
                  type="button"
                  title="Zoom out"
                  aria-label="Zoom out"
                  disabled={zoom <= MEDIA_ZOOM_MIN}
                  onClick={() => setZoom((value) => Math.max(MEDIA_ZOOM_MIN, value - MEDIA_ZOOM_STEP))}
                >−</button>
                <button
                  className="topbar-zoom-btn"
                  data-testid="media-zoom-increase"
                  type="button"
                  title="Zoom in"
                  aria-label="Zoom in"
                  disabled={zoom >= MEDIA_ZOOM_MAX}
                  onClick={() => setZoom((value) => Math.min(MEDIA_ZOOM_MAX, value + MEDIA_ZOOM_STEP))}
                >+</button>
                <button
                  className="topbar-zoom-reset"
                  data-testid="media-zoom-reset"
                  type="button"
                  disabled={zoom === 100}
                  onClick={() => setZoom(100)}
                >Reset</button>
              </div>
              {canOpenExternally && onOpenExternal && (
                <button
                  className="btn-ghost"
                  type="button"
                  onClick={() => onOpenExternal(item.url)}
                >
                  Open externally <ExternalLink size={14} />
                </button>
              )}
              <button
                className="icon-btn"
                data-testid="media-preview-close"
                type="button"
                title="Close"
                aria-label="Close media preview"
                onClick={() => setPreviewOpen(false)}
              >
                <X size={18} />
              </button>
            </div>
          </div>
          <div
            className={`generated-media-stage${zoom > 100 ? " is-zoomed" : ""}`}
            data-testid="generated-media-pan-stage"
            tabIndex={zoom > 100 ? 0 : -1}
            aria-label={zoom > 100 ? "Zoomed media canvas. Use the mouse wheel to zoom and drag to pan." : "Media canvas. Use the mouse wheel to zoom."}
            onPointerDown={startPan}
            onPointerMove={movePan}
            onPointerUp={endPan}
            onPointerCancel={endPan}
            onWheel={zoomWithWheel}
            onClick={closeFromBackdropClick}
            onLostPointerCapture={(event) => {
              panStart.current = null;
              event.currentTarget.classList.remove("is-panning");
            }}
          >
            <div className="generated-media-canvas" style={{ width: `${zoom}%`, height: `${zoom}%` }}>
              {video ? (
                <video src={source} controls autoPlay draggable={false} />
              ) : (
                <img src={source} alt={alt} draggable={false} />
              )}
            </div>
          </div>
        </SheetDialog>,
        document.body,
      )}
    </>
  );
}
