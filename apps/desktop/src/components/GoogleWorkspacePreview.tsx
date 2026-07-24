import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type FormEvent,
  type KeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";

import {
  chooseGoogleWorkspaceFiles,
  getGoogleFileContent,
  getGoogleFilePreview,
  getGoogleWorkspaceStatus,
  openExternalUrl,
  type GoogleFilePreview,
  type GoogleFileSummary,
  type GoogleWorkspaceStatus,
} from "../api";
import { googleWorkspaceFileUrl } from "../lib/googleWorkspace";
import { useContextMenu } from "./ContextMenu";
import { ArrowLeft, ArrowRight, Copy, ExternalLink, FileText, Folder, Plus, Refresh, Search, Sidebar } from "./icons";

export function GoogleWorkspacePreview({
  fileId,
  fallbackUrl,
  active,
  onMetadata,
  onOpenFile,
  onOpenFileInNewTab,
}: {
  fileId: string;
  fallbackUrl: string;
  active: boolean;
  onMetadata?: (title: string, faviconUrl?: string) => void;
  onOpenFile: (url: string) => void;
  onOpenFileInNewTab: (url: string) => void;
}) {
  const [preview, setPreview] = useState<GoogleFilePreview | null>(null);
  const [workspaceStatus, setWorkspaceStatus] = useState<GoogleWorkspaceStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [range, setRange] = useState("");
  const [requestedRange, setRequestedRange] = useState<string | undefined>();
  const [refreshKey, setRefreshKey] = useState(0);
  const loadedRequestRef = useRef<string | null>(null);
  const onMetadataRef = useRef(onMetadata);
  onMetadataRef.current = onMetadata;
  const isFolderUrl = /drive\.google\.com\/drive\/(?:u\/\d+\/)?folders\//i.test(fallbackUrl);

  const refresh = useCallback(() => setRefreshKey((value) => value + 1), []);

  useEffect(() => {
    if (!active) return;
    void getGoogleWorkspaceStatus().then(setWorkspaceStatus).catch(() => {});
  }, [active]);

  useEffect(() => {
    const requestKey = `${fileId}\0${requestedRange ?? ""}\0${refreshKey}`;
    if (!googleWorkspacePreviewNeedsLoad(active, loadedRequestRef.current, requestKey))
      return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    getGoogleFilePreview(fileId, requestedRange)
      .then((next) => {
        if (cancelled) return;
        loadedRequestRef.current = requestKey;
        setPreview(next);
        onMetadataRef.current?.(next.file.name, next.file.icon_link || undefined);
        if (next.kind === "sheet") setRange(next.range);
      })
      .catch((cause) => {
        if (!cancelled) {
          setPreview(null);
          setError(cause instanceof Error ? cause.message : String(cause));
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [active, fileId, refreshKey, requestedRange]);

  useEffect(() => {
    if (!active) return;
    const onFocus = () => refresh();
    const onToolResult = () => refresh();
    window.addEventListener("focus", onFocus);
    window.addEventListener("milim-google-workspace-refresh", onToolResult);
    return () => {
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("milim-google-workspace-refresh", onToolResult);
    };
  }, [active, refresh]);

  async function chooseFile() {
    setChoosing(true);
    setError(null);
    try {
      await chooseGoogleWorkspaceFiles([fileId]);
      refresh();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setChoosing(false);
    }
  }

  function submitRange(event: FormEvent) {
    event.preventDefault();
    setRequestedRange(range.trim() || undefined);
  }

  if (loading && !preview) {
    return (
      <div className="google-workspace-state" role="status">
        <span className="spinner" aria-hidden="true" />
        <strong>Loading from Google Drive...</strong>
      </div>
    );
  }

  if (!preview) {
    const unavailable = workspaceStatus?.available === false;
    return (
      <div className="google-workspace-state google-workspace-connect">
        <FileText size={30} aria-hidden="true" />
        <strong>
          {unavailable
            ? "Google Workspace isn't configured in this build"
            : isFolderUrl
              ? "Choose files from this folder"
              : "Choose this file with Google"}
        </strong>
        <span>
          {unavailable
            ? workspaceStatus.unavailable_reason
            : isFolderUrl
              ? "Open the folder in Google Picker, select its files, then choose Insert. Only those files become available to Milim."
              : "Google’s embedded editor is not reliable here. Milim uses Google Picker so only files you explicitly choose become available."}
        </span>
        {error && !unavailable ? <p className="sheet-hint error" role="alert">{error}</p> : null}
        <div className="google-workspace-actions">
          <button className="btn-accent" type="button" disabled={choosing || unavailable} onClick={() => void chooseFile()}>
            {choosing ? "Waiting for Google..." : isFolderUrl ? "Choose folder files" : "Choose with Google"}
          </button>
          <button className="btn-ghost" type="button" onClick={() => void openExternalUrl(fallbackUrl)}>
            <ExternalLink size={13} /> Open in Google
          </button>
        </div>
      </div>
    );
  }

  const externalUrl = googleWorkspaceFileUrl(preview.file) || fallbackUrl;
  return (
    <section className="google-workspace-preview" aria-label={`${preview.file.name} preview`}>
      <header className="google-workspace-preview-head">
        <div>
          <GoogleFileIcon file={preview.file} />
          <span>
            <strong>{preview.file.name}</strong>
            <small>{googleFileKindLabel(preview.file)}</small>
          </span>
        </div>
        <div className="google-workspace-actions">
          <button className="preview-browser-action" type="button" title="Refresh from Google" aria-label="Refresh from Google" disabled={loading} onClick={refresh}>
            <Refresh size={14} />
          </button>
          <button className="preview-browser-action" type="button" title="Open in Google" aria-label="Open in Google" onClick={() => void openExternalUrl(externalUrl)}>
            <ExternalLink size={14} />
          </button>
        </div>
      </header>
      {error ? <div className="preview-browser-error" role="alert">{error}</div> : null}
      {preview.kind === "sheet" ? (
        <SheetPreview
          preview={preview}
          range={range}
          setRange={setRange}
          loadRange={(value) => {
            setRange(value);
            setRequestedRange(value);
          }}
          submitRange={submitRange}
        />
      ) : preview.kind === "document" ? (
        <DocumentPreview document={preview.document} fallbackText={preview.text} />
      ) : preview.kind === "presentation" ? (
        <SlidesPreview fileId={preview.file.id} slides={preview.slides} active={active} />
      ) : preview.kind === "folder" ? (
        <FolderPreview
          children={preview.children}
          choosing={choosing}
          onChoose={() => void chooseFile()}
          onOpenFile={onOpenFile}
          onOpenFileInNewTab={onOpenFileInNewTab}
        />
      ) : preview.kind === "text" ? (
        <pre className="google-text-preview">{preview.text}</pre>
      ) : preview.kind === "unsupported" ? (
        <div className="google-workspace-state">
          <FileText size={30} aria-hidden="true" />
          <strong>No native viewer for this file type</strong>
          <span>Open it in Google or download it with an approved Drive transfer.</span>
        </div>
      ) : (
        <DriveMediaPreview file={preview.file} kind={preview.kind} active={active} />
      )}
    </section>
  );
}

export function googleWorkspacePreviewNeedsLoad(
  active: boolean,
  loadedRequest: string | null,
  request: string,
): boolean {
  return active && loadedRequest !== request;
}

export function SheetPreview({
  preview,
  range,
  setRange,
  loadRange,
  submitRange,
}: {
  preview: Extract<GoogleFilePreview, { kind: "sheet" }>;
  range: string;
  setRange: (value: string) => void;
  loadRange: (value: string) => void;
  submitRange: (event: FormEvent) => void;
}) {
  const sheetTitles = preview.sheets
    .map((sheet) => {
      const properties = isRecord(sheet) && isRecord(sheet.properties) ? sheet.properties : null;
      return properties && typeof properties.title === "string" ? properties.title : null;
    })
    .filter((title): title is string => Boolean(title));
  const width = Math.max(
    1,
    ...preview.values.map((row) => row.length),
    ...preview.formulas.map((row) => row.length),
  );
  const gridRef = useRef<HTMLDivElement>(null);
  const [selectedCell, setSelectedCell] = useState<[number, number]>([0, 0]);
  const [columnWidths, setColumnWidths] = useState(() => sheetColumnWidths(preview, width));
  const [wrapCells, setWrapCells] = useState(false);
  const [search, setSearch] = useState("");
  const [zoom, setZoom] = useState(100);
  const activeSheet = sheetTitleFromRange(preview.range);
  const rangeOrigin = sheetRangeOrigin(preview.range);
  const [selectedRow, selectedColumn] = selectedCell;
  const activeValue = displayCell(preview.values[selectedRow]?.[selectedColumn]);
  const activeFormula = displayCell(preview.formulas[selectedRow]?.[selectedColumn]);
  const activeContent = activeFormula.startsWith("=") ? activeFormula : activeValue;
  const activeAddress = `${columnLabel(rangeOrigin.column + selectedColumn)}${rangeOrigin.row + selectedRow}`;
  const matches = useMemo(() => {
    const needle = search.trim().toLocaleLowerCase();
    if (!needle) return [] as Array<[number, number]>;
    const found: Array<[number, number]> = [];
    preview.values.forEach((row, rowIndex) => {
      for (let columnIndex = 0; columnIndex < width; columnIndex += 1) {
        const value = `${displayCell(row[columnIndex])}\n${displayCell(preview.formulas[rowIndex]?.[columnIndex])}`;
        if (value.toLocaleLowerCase().includes(needle)) found.push([rowIndex, columnIndex]);
      }
    });
    return found;
  }, [preview.formulas, preview.values, search, width]);
  const matchKeys = useMemo(
    () => new Set(matches.map(([row, column]) => `${row}:${column}`)),
    [matches],
  );

  useEffect(() => {
    setSelectedCell([0, 0]);
    setColumnWidths(sheetColumnWidths(preview, width));
  }, [preview.range, width]);

  useEffect(() => {
    gridRef.current
      ?.querySelector<HTMLElement>(`[data-sheet-cell="${selectedRow}:${selectedColumn}"]`)
      ?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [selectedColumn, selectedRow]);

  function selectCell(row: number, column: number) {
    setSelectedCell([
      Math.max(0, Math.min(preview.values.length - 1, row)),
      Math.max(0, Math.min(width - 1, column)),
    ]);
  }

  function selectNextMatch() {
    if (!matches.length) return;
    const current = matches.findIndex(([row, column]) => row === selectedRow && column === selectedColumn);
    setSelectedCell(matches[(current + 1) % matches.length]);
    gridRef.current?.focus();
  }

  function handleGridKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (["INPUT", "BUTTON", "SELECT"].includes((event.target as HTMLElement).tagName)) return;
    const offsets: Record<string, [number, number]> = {
      ArrowUp: [-1, 0],
      ArrowDown: [1, 0],
      ArrowLeft: [0, -1],
      ArrowRight: [0, 1],
    };
    const offset = offsets[event.key];
    if (!offset) return;
    event.preventDefault();
    selectCell(selectedRow + offset[0], selectedColumn + offset[1]);
  }

  function resizeColumn(index: number, delta: number) {
    setColumnWidths((current) => current.map((value, column) => (
      column === index ? Math.max(64, Math.min(480, value + delta)) : value
    )));
  }

  function startColumnResize(event: ReactPointerEvent<HTMLButtonElement>, index: number) {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = columnWidths[index];
    const move = (moveEvent: PointerEvent) => {
      setColumnWidths((current) => current.map((value, column) => (
        column === index
          ? Math.max(64, Math.min(480, startWidth + moveEvent.clientX - startX))
          : value
      )));
    };
    const stop = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", stop);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", stop);
  }

  return (
    <div className="google-sheet-preview">
      <form className="google-sheet-range" onSubmit={submitRange}>
        <label>
          <span>Range</span>
          <input value={range} onChange={(event) => setRange(event.currentTarget.value)} aria-label="Google Sheets range" />
        </label>
        <button className="btn-ghost" type="submit">Load</button>
      </form>
      <div className="google-sheet-tabs" aria-label="Worksheets">
        {sheetTitles.map((title) => (
          <button
            type="button"
            key={title}
            className={title === activeSheet ? "active" : ""}
            aria-current={title === activeSheet ? "page" : undefined}
            onClick={() => loadRange(`'${title.split("'").join("''")}'!A1:Z100`)}
          >
            {title}
          </button>
        ))}
      </div>
      <div className="google-sheet-toolbar">
        <label className="google-sheet-search">
          <input
            type="search"
            value={search}
            placeholder="Search this range"
            aria-label="Search this sheet range"
            onChange={(event) => setSearch(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                selectNextMatch();
              }
            }}
          />
          {search ? <small>{matches.length} {matches.length === 1 ? "match" : "matches"}</small> : null}
        </label>
        <button className={wrapCells ? "active" : ""} type="button" aria-pressed={wrapCells} onClick={() => setWrapCells((value) => !value)}>
          Wrap
        </button>
        <label className="google-sheet-zoom">
          <select value={zoom} aria-label="Sheet zoom" onChange={(event) => setZoom(Number(event.currentTarget.value))}>
            {[75, 100, 125, 150].map((value) => <option key={value} value={value}>{value}%</option>)}
          </select>
        </label>
      </div>
      <div className="google-sheet-formula-bar" aria-live="polite">
        <output aria-label="Active cell">{activeAddress}</output>
        <span title={activeContent}>{activeContent || "\u00a0"}</span>
      </div>
      <div
        className="google-sheet-grid-wrap"
        ref={gridRef}
        tabIndex={0}
        aria-label="Spreadsheet grid. Use arrow keys to move between cells."
        onKeyDown={handleGridKeyDown}
      >
        <table
          className={`google-sheet-grid${wrapCells ? " is-wrapped" : ""}`}
          style={{ "--sheet-zoom": zoom / 100 } as CSSProperties}
        >
          <colgroup>
            <col className="google-sheet-row-column" />
            {columnWidths.map((value, index) => <col key={index} style={{ width: value }} />)}
          </colgroup>
          <thead>
            <tr>
              <th aria-label="Row" />
              {Array.from({ length: width }, (_, index) => (
                <th key={index}>
                  {columnLabel(rangeOrigin.column + index)}
                  <button
                    className="google-sheet-column-resize"
                    type="button"
                    aria-label={`Resize column ${columnLabel(rangeOrigin.column + index)}`}
                    onPointerDown={(event) => startColumnResize(event, index)}
                    onKeyDown={(event) => {
                      if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
                        event.preventDefault();
                        resizeColumn(index, event.key === "ArrowLeft" ? -16 : 16);
                      }
                    }}
                  />
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {preview.values.length ? preview.values.map((row, rowIndex) => (
              <tr key={rowIndex}>
                <th>{rangeOrigin.row + rowIndex}</th>
                {Array.from({ length: width }, (_, columnIndex) => {
                  const formula = displayCell(preview.formulas[rowIndex]?.[columnIndex]);
                  const active = rowIndex === selectedRow && columnIndex === selectedColumn;
                  const match = matchKeys.has(`${rowIndex}:${columnIndex}`);
                  return (
                    <td
                      key={columnIndex}
                      className={`${active ? "active" : ""}${match ? " match" : ""}`.trim()}
                      aria-selected={active}
                      data-sheet-cell={`${rowIndex}:${columnIndex}`}
                      onClick={() => {
                        selectCell(rowIndex, columnIndex);
                        gridRef.current?.focus();
                      }}
                    >
                      <span>{displayCell(row[columnIndex])}</span>
                      {formula.startsWith("=") ? <code>{formula}</code> : null}
                    </td>
                  );
                })}
              </tr>
            )) : (
              <tr><td colSpan={width + 1}>This range is empty.</td></tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}

export function DocumentPreview({
  document,
  fallbackText,
}: {
  document: Record<string, unknown>;
  fallbackText: string;
}) {
  const [query, setQuery] = useState("");
  const [matchCount, setMatchCount] = useState(0);
  const [activeMatch, setActiveMatch] = useState(0);
  const [zoom, setZoom] = useState(100);
  const [fitWidth, setFitWidth] = useState(true);
  const [outlineOpen, setOutlineOpen] = useState(false);
  const contentRef = useRef<HTMLDivElement | null>(null);
  const body = isRecord(document.body) && Array.isArray(document.body.content)
    ? document.body.content
    : [];
  const inlineObjects = isRecord(document.inlineObjects)
    ? document.inlineObjects
    : {};
  const lists = isRecord(document.lists) ? document.lists : {};
  const outline = useMemo(() => googleDocOutline(body), [body]);
  const normalizedQuery = query.trim();

  useEffect(() => {
    const matches = Array.from(contentRef.current?.querySelectorAll<HTMLElement>("[data-doc-search-match]") ?? []);
    setMatchCount(matches.length);
    const nextActive = matches.length ? Math.min(activeMatch, matches.length - 1) : 0;
    matches.forEach((match, index) => match.classList.toggle("active", index === nextActive));
    matches[nextActive]?.scrollIntoView({ block: "center" });
  }, [activeMatch, document, normalizedQuery]);

  function moveSearchMatch(delta: -1 | 1) {
    if (!matchCount) return;
    setActiveMatch((current) => (current + delta + matchCount) % matchCount);
  }

  return (
    <div className="google-doc-viewer">
      <div className="google-viewer-toolbar">
        <label className="google-viewer-search">
          <Search size={13} aria-hidden="true" />
          <input
            aria-label="Search document"
            placeholder="Find in document"
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setActiveMatch(0);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                moveSearchMatch(event.shiftKey ? -1 : 1);
              }
            }}
          />
        </label>
        <span className="google-viewer-match-count" aria-live="polite">
          {normalizedQuery ? `${matchCount ? activeMatch + 1 : 0}/${matchCount}` : ""}
        </span>
        <button className="preview-browser-action" type="button" aria-label="Previous document match" disabled={!matchCount} onClick={() => moveSearchMatch(-1)}>
          <ArrowLeft size={13} />
        </button>
        <button className="preview-browser-action" type="button" aria-label="Next document match" disabled={!matchCount} onClick={() => moveSearchMatch(1)}>
          <ArrowRight size={13} />
        </button>
        <span className="google-viewer-toolbar-spacer" />
        <button
          className={`preview-browser-action${outlineOpen ? " active" : ""}`}
          type="button"
          aria-label={outlineOpen ? "Hide document outline" : "Show document outline"}
          aria-pressed={outlineOpen}
          disabled={!outline.length}
          onClick={() => setOutlineOpen((value) => !value)}
        >
          <Sidebar size={13} />
        </button>
        <button className={`google-viewer-text-button${fitWidth ? " active" : ""}`} type="button" aria-pressed={fitWidth} onClick={() => setFitWidth((value) => !value)}>
          Fit width
        </button>
        <label className="google-viewer-zoom">
          <span>Zoom</span>
          <select aria-label="Document zoom" value={zoom} onChange={(event) => setZoom(Number(event.target.value))}>
            {[75, 90, 100, 110, 125, 150].map((value) => <option value={value} key={value}>{value}%</option>)}
          </select>
        </label>
      </div>
      <div className="google-doc-body">
        {outlineOpen && outline.length ? (
          <nav className="google-doc-outline" aria-label="Document outline">
            <strong>Outline</strong>
            {outline.map((item) => (
              <button
                type="button"
                key={item.index}
                style={{ paddingLeft: 8 + item.level * 8 }}
                onClick={() => contentRef.current?.querySelector(`[data-doc-heading="${item.index}"]`)?.scrollIntoView({ block: "start" })}
              >
                {item.text}
              </button>
            ))}
          </nav>
        ) : null}
        <div className="google-doc-canvas" ref={contentRef}>
          <article
            className={`google-doc-preview${fitWidth ? " fit-width" : ""}`}
            style={{ "--doc-zoom": zoom / 100 } as CSSProperties}
          >
            {body.length ? body.map((item, index) => (
              <DocStructuralElement
                value={item}
                inlineObjects={inlineObjects}
                lists={lists}
                query={normalizedQuery}
                blockIndex={index}
                key={index}
              />
            )) : (
              <p>{fallbackText || "This document is empty."}</p>
            )}
          </article>
        </div>
      </div>
    </div>
  );
}

function DocStructuralElement({
  value,
  inlineObjects,
  lists,
  query,
  blockIndex,
}: {
  value: unknown;
  inlineObjects: Record<string, unknown>;
  lists: Record<string, unknown>;
  query: string;
  blockIndex?: number;
}): ReactNode {
  if (!isRecord(value)) return null;
  if (isRecord(value.paragraph) && Array.isArray(value.paragraph.elements)) {
    const style = isRecord(value.paragraph.paragraphStyle) ? value.paragraph.paragraphStyle : {};
    const namedStyle = typeof style.namedStyleType === "string" ? style.namedStyleType : "";
    const content = value.paragraph.elements.map((element, index) => (
      <DocParagraphElement value={element} inlineObjects={inlineObjects} query={query} key={index} />
    ));
    const paragraphStyle = googleDocParagraphStyle(style);
    const headingProps = blockIndex === undefined ? {} : { "data-doc-heading": blockIndex };
    if (namedStyle === "TITLE") return <h1 style={paragraphStyle} {...headingProps}>{content}</h1>;
    if (namedStyle === "SUBTITLE") return <p className="google-doc-subtitle" style={paragraphStyle}>{content}</p>;
    if (namedStyle.startsWith("HEADING_")) {
      const level = Math.min(6, Math.max(2, Number(namedStyle.slice(-1)) + 1));
      if (level === 2) return <h2 style={paragraphStyle} {...headingProps}>{content}</h2>;
      if (level === 3) return <h3 style={paragraphStyle} {...headingProps}>{content}</h3>;
      if (level === 4) return <h4 style={paragraphStyle} {...headingProps}>{content}</h4>;
      if (level === 5) return <h5 style={paragraphStyle} {...headingProps}>{content}</h5>;
      return <h6 style={paragraphStyle} {...headingProps}>{content}</h6>;
    }
    if (isRecord(value.paragraph.bullet)) {
      const bullet = value.paragraph.bullet;
      const nestingLevel = typeof bullet.nestingLevel === "number" ? bullet.nestingLevel : 0;
      return (
        <p
          className="google-doc-list-item"
          data-list-kind={googleDocListKind(bullet, lists)}
          style={{ ...paragraphStyle, paddingLeft: `${18 + nestingLevel * 18}px` }}
        >
          {content}
        </p>
      );
    }
    return <p style={paragraphStyle}>{content}</p>;
  }
  if (isRecord(value.table) && Array.isArray(value.table.tableRows)) {
    return (
      <table>
        <tbody>
          {value.table.tableRows.map((row, rowIndex) => (
            <tr key={rowIndex}>
              {isRecord(row) && Array.isArray(row.tableCells) ? row.tableCells.map((cell, cellIndex) => (
                <td key={cellIndex}>
                  {isRecord(cell) && Array.isArray(cell.content)
                    ? cell.content.map((item, index) => (
                      <DocStructuralElement value={item} inlineObjects={inlineObjects} lists={lists} query={query} key={index} />
                    ))
                    : null}
                </td>
              )) : null}
            </tr>
          ))}
        </tbody>
      </table>
    );
  }
  return null;
}

function DocParagraphElement({
  value,
  inlineObjects,
  query,
}: {
  value: unknown;
  inlineObjects: Record<string, unknown>;
  query: string;
}): ReactNode {
  if (!isRecord(value)) return null;
  if (isRecord(value.pageBreak)) {
    return <hr className="google-doc-page-break" />;
  }
  if (isRecord(value.inlineObjectElement) && typeof value.inlineObjectElement.inlineObjectId === "string") {
    const object = inlineObjects[value.inlineObjectElement.inlineObjectId];
    const properties = isRecord(object) && isRecord(object.inlineObjectProperties)
      ? object.inlineObjectProperties
      : null;
    const embedded = properties && isRecord(properties.embeddedObject)
      ? properties.embeddedObject
      : null;
    const image = embedded && isRecord(embedded.imageProperties)
      ? embedded.imageProperties
      : null;
    return image && typeof image.contentUri === "string"
      ? <img className="google-doc-inline-image" src={image.contentUri} alt="" />
      : null;
  }
  if (!isRecord(value.textRun) || typeof value.textRun.content !== "string") return null;
  const style = isRecord(value.textRun.textStyle) ? value.textRun.textStyle : {};
  const textStyle = googleDocTextStyle(style);
  return (
    <span className={isRecord(style.link) ? "google-doc-link" : undefined} style={textStyle}>
      {highlightGoogleDocText(value.textRun.content, query)}
    </span>
  );
}

export function SlidesPreview({
  fileId,
  slides,
  active,
}: {
  fileId: string;
  slides: Array<{ objectId?: string | null; text: string; notes?: string | null }>;
  active: boolean;
}) {
  const [selected, setSelected] = useState(0);
  const slide = slides[selected] ?? slides[0];
  const [thumbnail, setThumbnail] = useState<string | null>(null);
  const [thumbnailError, setThumbnailError] = useState<string | null>(null);
  const thumbnailObjectUrlRef = useRef<string | null>(null);
  const thumbnailRequestRef = useRef<string | null>(null);
  const [query, setQuery] = useState("");
  const [zoom, setZoom] = useState(100);
  const [fit, setFit] = useState(true);
  const [notesOpen, setNotesOpen] = useState(false);
  const matchingSlides = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    if (!normalized) return [];
    return slides.flatMap((item, index) => `${item.text}\n${item.notes ?? ""}`.toLocaleLowerCase().includes(normalized) ? [index] : []);
  }, [query, slides]);

  useEffect(() => {
    if (selected < slides.length) return;
    setSelected(Math.max(0, slides.length - 1));
  }, [selected, slides.length]);

  useEffect(() => {
    if (!slide?.objectId) return;
    const requestKey = `${fileId}\0${slide.objectId}`;
    if (!googleWorkspacePreviewNeedsLoad(active, thumbnailRequestRef.current, requestKey))
      return;
    thumbnailRequestRef.current = requestKey;
    let cancelled = false;
    let objectUrl: string | null = null;
    setThumbnail(null);
    setThumbnailError(null);
    getGoogleFileContent(fileId, slide.objectId)
      .then((blob) => {
        if (cancelled) return;
        objectUrl = URL.createObjectURL(blob);
        if (thumbnailObjectUrlRef.current)
          URL.revokeObjectURL(thumbnailObjectUrlRef.current);
        thumbnailObjectUrlRef.current = objectUrl;
        setThumbnail(objectUrl);
      })
      .catch((cause) => {
        if (!cancelled) setThumbnailError(cause instanceof Error ? cause.message : String(cause));
      });
    return () => {
      cancelled = true;
      if (!objectUrl && thumbnailRequestRef.current === requestKey)
        thumbnailRequestRef.current = null;
    };
  }, [active, fileId, slide?.objectId]);

  useEffect(() => () => {
    if (thumbnailObjectUrlRef.current)
      URL.revokeObjectURL(thumbnailObjectUrlRef.current);
  }, []);

  if (!slides.length) return <div className="google-workspace-state"><strong>This presentation is empty.</strong></div>;

  function selectRelative(delta: -1 | 1) {
    setSelected((current) => Math.min(slides.length - 1, Math.max(0, current + delta)));
  }

  function selectNextMatch(reverse = false) {
    if (!matchingSlides.length) return;
    const current = matchingSlides.indexOf(selected);
    const next = reverse
      ? (current <= 0 ? matchingSlides.length : current) - 1
      : (current + 1) % matchingSlides.length;
    setSelected(matchingSlides[next]);
  }

  return (
    <div
      className="google-slides-viewer"
      tabIndex={0}
      aria-label="Presentation viewer. Use arrow keys to change slides."
      onKeyDown={(event) => {
        if (event.target instanceof HTMLInputElement || event.target instanceof HTMLSelectElement) return;
        if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
          event.preventDefault();
          selectRelative(-1);
        } else if (event.key === "ArrowRight" || event.key === "ArrowDown") {
          event.preventDefault();
          selectRelative(1);
        } else if (event.key === "Home") {
          event.preventDefault();
          setSelected(0);
        } else if (event.key === "End") {
          event.preventDefault();
          setSelected(slides.length - 1);
        }
      }}
    >
      <div className="google-viewer-toolbar">
        <button className="preview-browser-action" type="button" aria-label="Previous slide" disabled={selected === 0} onClick={() => selectRelative(-1)}>
          <ArrowLeft size={13} />
        </button>
        <strong className="google-slide-counter">{selected + 1} / {slides.length}</strong>
        <button className="preview-browser-action" type="button" aria-label="Next slide" disabled={selected === slides.length - 1} onClick={() => selectRelative(1)}>
          <ArrowRight size={13} />
        </button>
        <label className="google-viewer-search">
          <Search size={13} aria-hidden="true" />
          <input
            aria-label="Search slides"
            placeholder="Find in slides"
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              const normalized = event.target.value.trim().toLocaleLowerCase();
              const firstMatch = slides.findIndex((item) => `${item.text}\n${item.notes ?? ""}`.toLocaleLowerCase().includes(normalized));
              if (normalized && firstMatch >= 0) setSelected(firstMatch);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                selectNextMatch(event.shiftKey);
              }
            }}
          />
        </label>
        <span className="google-viewer-match-count" aria-live="polite">
          {query.trim() ? `${matchingSlides.length} match${matchingSlides.length === 1 ? "" : "es"}` : ""}
        </span>
        <span className="google-viewer-toolbar-spacer" />
        {slides.some((item) => item.notes?.trim()) ? (
          <button className={`google-viewer-text-button${notesOpen ? " active" : ""}`} type="button" aria-pressed={notesOpen} onClick={() => setNotesOpen((value) => !value)}>
            Notes
          </button>
        ) : null}
        <button className={`google-viewer-text-button${fit ? " active" : ""}`} type="button" aria-pressed={fit} onClick={() => setFit((value) => !value)}>
          Fit
        </button>
        <label className="google-viewer-zoom">
          <span>Zoom</span>
          <select aria-label="Slide zoom" value={zoom} disabled={fit} onChange={(event) => setZoom(Number(event.target.value))}>
            {[75, 100, 125, 150, 200].map((value) => <option value={value} key={value}>{value}%</option>)}
          </select>
        </label>
      </div>
      <div className={`google-slides-preview${notesOpen ? " notes-open" : ""}`}>
      <div className="google-slide-rail">
        {slides.map((item, index) => (
          <button
            className={`${index === selected ? "active" : ""}${matchingSlides.includes(index) ? " match" : ""}`.trim()}
            type="button"
            key={item.objectId ?? index}
            aria-current={index === selected ? "page" : undefined}
            onClick={() => setSelected(index)}
          >
            <span className="google-slide-rail-number">{index + 1}</span>
            <SlideRailThumbnail
              fileId={fileId}
              objectId={item.objectId}
              active={active}
              selectedSource={index === selected ? thumbnail : null}
            />
            <small>{item.text.trim().split("\n")[0]?.slice(0, 64) || "Untitled slide"}</small>
          </button>
        ))}
      </div>
      <div className="google-slide-stage">
        {thumbnail ? <img className={fit ? "fit" : ""} style={fit ? undefined : { width: `${zoom}%` }} src={thumbnail} alt={`Slide ${selected + 1}`} /> : (
          <div className="google-slide-fallback">
            {!thumbnailError ? <span className="spinner" aria-hidden="true" /> : null}
            <strong>Slide {selected + 1}</strong>
            <p>{slide?.text || "No text on this slide."}</p>
          </div>
        )}
        {thumbnailError ? <p className="sheet-hint error">{thumbnailError}</p> : null}
      </div>
      {notesOpen ? (
        <aside className="google-slide-notes">
          <strong>Speaker notes</strong>
          <p>{slide?.notes?.trim() || "No speaker notes on this slide."}</p>
        </aside>
      ) : null}
      </div>
    </div>
  );
}

function SlideRailThumbnail({
  fileId,
  objectId,
  active,
  selectedSource,
}: {
  fileId: string;
  objectId?: string | null;
  active: boolean;
  selectedSource: string | null;
}) {
  const rootRef = useRef<HTMLSpanElement | null>(null);
  const [visible, setVisible] = useState(false);
  const [source, setSource] = useState<string | null>(null);
  const objectUrlRef = useRef<string | null>(null);
  const requestRef = useRef<string | null>(null);

  useEffect(() => {
    const element = rootRef.current;
    if (!element || visible) return;
    if (typeof IntersectionObserver === "undefined") {
      setVisible(true);
      return;
    }
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) {
        setVisible(true);
        observer.disconnect();
      }
    }, { rootMargin: "120px" });
    observer.observe(element);
    return () => observer.disconnect();
  }, [visible]);

  useEffect(() => {
    if (!visible || !objectId || selectedSource) return;
    const requestKey = `${fileId}\0${objectId}`;
    if (!googleWorkspacePreviewNeedsLoad(active, requestRef.current, requestKey))
      return;
    requestRef.current = requestKey;
    let cancelled = false;
    let objectUrl: string | null = null;
    getGoogleFileContent(fileId, objectId)
      .then((blob) => {
        if (cancelled) return;
        objectUrl = URL.createObjectURL(blob);
        if (objectUrlRef.current) URL.revokeObjectURL(objectUrlRef.current);
        objectUrlRef.current = objectUrl;
        setSource(objectUrl);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      if (!objectUrl && requestRef.current === requestKey)
        requestRef.current = null;
    };
  }, [active, fileId, objectId, selectedSource, visible]);

  useEffect(() => () => {
    if (objectUrlRef.current) URL.revokeObjectURL(objectUrlRef.current);
  }, []);

  const thumbnailSource = selectedSource || source;
  return (
    <span className="google-slide-rail-thumbnail" ref={rootRef}>
      {thumbnailSource ? <img src={thumbnailSource} alt="" /> : <span aria-hidden="true" />}
    </span>
  );
}

function googleDocOutline(body: unknown[]): Array<{ index: number; level: number; text: string }> {
  return body.flatMap((item, index) => {
    if (!isRecord(item) || !isRecord(item.paragraph)) return [];
    const style = isRecord(item.paragraph.paragraphStyle) ? item.paragraph.paragraphStyle : {};
    const namedStyle = typeof style.namedStyleType === "string" ? style.namedStyleType : "";
    if (namedStyle !== "TITLE" && !namedStyle.startsWith("HEADING_")) return [];
    const elements = Array.isArray(item.paragraph.elements) ? item.paragraph.elements : [];
    const text = elements
      .map((element) => isRecord(element) && isRecord(element.textRun) && typeof element.textRun.content === "string" ? element.textRun.content : "")
      .join("")
      .trim();
    if (!text) return [];
    return [{
      index,
      level: namedStyle === "TITLE" ? 0 : Math.max(1, Number(namedStyle.slice(-1)) || 1),
      text,
    }];
  });
}

function googleDocParagraphStyle(style: Record<string, unknown>): CSSProperties {
  const alignment = typeof style.alignment === "string" ? style.alignment.toLocaleLowerCase() : undefined;
  const direction = style.contentDirection === "RIGHT_TO_LEFT" ? "rtl" : undefined;
  const lineSpacing = typeof style.lineSpacing === "number" ? style.lineSpacing / 100 : undefined;
  return {
    direction,
    textAlign: alignment === "start" || alignment === "end" || alignment === "center" || alignment === "justify" ? alignment : undefined,
    lineHeight: lineSpacing,
    marginTop: googleDocDimension(style.spaceAbove),
    marginBottom: googleDocDimension(style.spaceBelow),
    paddingLeft: googleDocDimension(style.indentStart),
    paddingRight: googleDocDimension(style.indentEnd),
  };
}

function googleDocTextStyle(style: Record<string, unknown>): CSSProperties {
  const fontSize = googleDocDimension(style.fontSize);
  const weightedFont = isRecord(style.weightedFontFamily) ? style.weightedFontFamily : {};
  const decorations = [
    style.underline === true ? "underline" : "",
    style.strikethrough === true ? "line-through" : "",
  ].filter(Boolean).join(" ");
  const baseline = style.baselineOffset === "SUPERSCRIPT"
    ? "super"
    : style.baselineOffset === "SUBSCRIPT"
      ? "sub"
      : undefined;
  return {
    color: googleDocColor(style.foregroundColor),
    backgroundColor: googleDocColor(style.backgroundColor),
    fontFamily: typeof weightedFont.fontFamily === "string" ? weightedFont.fontFamily : undefined,
    fontSize,
    fontWeight: style.bold === true ? 700 : undefined,
    fontStyle: style.italic === true ? "italic" : undefined,
    textDecoration: decorations || undefined,
    verticalAlign: baseline,
  };
}

function googleDocDimension(value: unknown): string | undefined {
  if (!isRecord(value) || typeof value.magnitude !== "number") return undefined;
  const unit = value.unit === "PT" ? "pt" : value.unit === "PX" ? "px" : "pt";
  return `calc(${value.magnitude}${unit} * var(--doc-zoom))`;
}

function googleDocColor(value: unknown): string | undefined {
  const color = isRecord(value) && isRecord(value.color) ? value.color : value;
  const rgb = isRecord(color) && isRecord(color.rgbColor) ? color.rgbColor : color;
  if (!isRecord(rgb)) return undefined;
  if (!["red", "green", "blue"].some((key) => typeof rgb[key] === "number")) return undefined;
  const channel = (key: string) => {
    const value = rgb[key];
    return Math.round(Math.min(1, Math.max(0, typeof value === "number" ? value : 0)) * 255);
  };
  return `rgb(${channel("red")} ${channel("green")} ${channel("blue")})`;
}

function googleDocListKind(bullet: Record<string, unknown>, lists: Record<string, unknown>): "ordered" | "unordered" {
  const candidate = typeof bullet.listId === "string" ? lists[bullet.listId] : null;
  const list = isRecord(candidate) ? candidate : null;
  const properties = list && isRecord(list.listProperties) ? list.listProperties : null;
  const nestingLevels = properties && Array.isArray(properties.nestingLevels) ? properties.nestingLevels : [];
  const level = typeof bullet.nestingLevel === "number" ? bullet.nestingLevel : 0;
  const nesting = isRecord(nestingLevels[level]) ? nestingLevels[level] : {};
  const glyphType = typeof nesting.glyphType === "string" ? nesting.glyphType : "";
  return /DECIMAL|ALPHA|ROMAN/.test(glyphType) ? "ordered" : "unordered";
}

function highlightGoogleDocText(text: string, query: string): ReactNode {
  if (!query) return text;
  const normalizedText = text.toLocaleLowerCase();
  const normalizedQuery = query.toLocaleLowerCase();
  const parts: ReactNode[] = [];
  let cursor = 0;
  let index = normalizedText.indexOf(normalizedQuery);
  // ponytail: highlights stay within text-style runs; merge runs only if cross-style matches become important.
  while (index >= 0) {
    if (index > cursor) parts.push(text.slice(cursor, index));
    parts.push(<mark data-doc-search-match key={`${index}-${parts.length}`}>{text.slice(index, index + query.length)}</mark>);
    cursor = index + query.length;
    index = normalizedText.indexOf(normalizedQuery, cursor);
  }
  if (cursor < text.length) parts.push(text.slice(cursor));
  return parts;
}

export function FolderPreview({
  children,
  choosing,
  onChoose,
  onOpenFile,
  onOpenFileInNewTab,
}: {
  children: GoogleFileSummary[];
  choosing: boolean;
  onChoose: () => void;
  onOpenFile: (url: string) => void;
  onOpenFileInNewTab: (url: string) => void;
}) {
  const { openContextMenu } = useContextMenu();

  function openFileContextMenu(event: ReactMouseEvent, file: GoogleFileSummary) {
    const url = googleWorkspaceFileUrl(file);
    openContextMenu(event, [
      {
        id: "open-google-file",
        label: "Open link",
        icon: <FileText size={13} />,
        action: () => onOpenFile(url),
      },
      {
        id: "open-google-file-new-tab",
        label: "Open link in new tab",
        icon: <Plus size={13} />,
        action: () => onOpenFileInNewTab(url),
      },
      {
        id: "copy-google-file-link",
        label: "Copy link",
        icon: <Copy size={13} />,
        separatorBefore: true,
        action: () => void navigator.clipboard?.writeText(url),
      },
      {
        id: "open-google-file-external",
        label: "Open in Google",
        icon: <ExternalLink size={13} />,
        action: () => void openExternalUrl(url),
      },
    ], file.name);
  }

  return (
    <div className="google-folder-preview">
      <div className="google-folder-authorize">
        <span>Select all the files you want in the single Google Picker.</span>
        <button className="btn-accent" type="button" disabled={choosing} onClick={onChoose}>
          {choosing ? "Waiting for Google..." : "Choose files"}
        </button>
      </div>
      {children.length ? children.map((file) => (
        <div className="google-folder-entry" key={file.id} onContextMenu={(event) => openFileContextMenu(event, file)}>
          <button
            className="google-folder-entry-main"
            type="button"
            onClick={(event) => {
              const url = googleWorkspaceFileUrl(file);
              if (browserLinkOpensNewTab(event)) onOpenFileInNewTab(url);
              else onOpenFile(url);
            }}
            onAuxClick={(event) => {
              if (!browserLinkOpensNewTab(event)) return;
              event.preventDefault();
              onOpenFileInNewTab(googleWorkspaceFileUrl(file));
            }}
          >
            <GoogleFileIcon file={file} />
            <span><strong>{file.name}</strong><small>{googleFileKindLabel(file)}</small></span>
          </button>
          <button
            className="preview-browser-action"
            type="button"
            title="Open in Google"
            aria-label={`Open ${file.name} in Google`}
            onClick={() => void openExternalUrl(googleWorkspaceFileUrl(file))}
          >
            <ExternalLink size={13} aria-hidden="true" />
          </button>
        </div>
      )) : <div className="google-workspace-state"><Folder size={30} /><strong>No files from this folder are authorized yet.</strong></div>}
    </div>
  );
}

export function browserLinkOpensNewTab(event: Pick<ReactMouseEvent, "button" | "ctrlKey" | "metaKey">): boolean {
  return event.button === 1 || event.ctrlKey || event.metaKey;
}

function DriveMediaPreview({
  file,
  kind,
  active,
}: {
  file: GoogleFileSummary;
  kind: "image" | "pdf" | "audio" | "video";
  active: boolean;
}) {
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const objectUrlRef = useRef<string | null>(null);
  const requestRef = useRef<string | null>(null);
  useEffect(() => {
    if (!googleWorkspacePreviewNeedsLoad(active, requestRef.current, file.id))
      return;
    requestRef.current = file.id;
    let cancelled = false;
    let objectUrl: string | null = null;
    setUrl(null);
    setError(null);
    getGoogleFileContent(file.id)
      .then((blob) => {
        if (cancelled) return;
        objectUrl = URL.createObjectURL(blob);
        if (objectUrlRef.current) URL.revokeObjectURL(objectUrlRef.current);
        objectUrlRef.current = objectUrl;
        setUrl(objectUrl);
      })
      .catch((cause) => {
        if (!cancelled) setError(cause instanceof Error ? cause.message : String(cause));
      });
    return () => {
      cancelled = true;
      if (!objectUrl && requestRef.current === file.id)
        requestRef.current = null;
    };
  }, [active, file.id]);
  useEffect(() => () => {
    if (objectUrlRef.current) URL.revokeObjectURL(objectUrlRef.current);
  }, []);
  if (error) return <div className="google-workspace-state"><p className="sheet-hint error">{error}</p></div>;
  if (!url) return <div className="google-workspace-state" role="status"><span className="spinner" /><strong>Loading preview...</strong></div>;
  if (kind === "image") return <div className="google-media-preview"><img src={url} alt={file.name} /></div>;
  if (kind === "pdf") return <iframe className="google-pdf-preview" src={url} title={file.name} />;
  if (kind === "audio") return <div className="google-media-preview"><audio controls src={url} /></div>;
  return <div className="google-media-preview"><video controls src={url} /></div>;
}

function GoogleFileIcon({ file }: { file: GoogleFileSummary }) {
  if (file.icon_link) return <img className="google-file-icon" src={file.icon_link} alt="" />;
  return file.mime_type === "application/vnd.google-apps.folder"
    ? <Folder size={16} aria-hidden="true" />
    : <FileText size={16} aria-hidden="true" />;
}

function googleFileKindLabel(file: GoogleFileSummary): string {
  switch (file.mime_type) {
    case "application/vnd.google-apps.spreadsheet": return "Google Sheets";
    case "application/vnd.google-apps.document": return "Google Docs";
    case "application/vnd.google-apps.presentation": return "Google Slides";
    case "application/vnd.google-apps.folder": return "Google Drive folder";
    default: return file.mime_type;
  }
}

function columnLabel(index: number): string {
  let value = index + 1;
  let label = "";
  while (value > 0) {
    value -= 1;
    label = String.fromCharCode(65 + (value % 26)) + label;
    value = Math.floor(value / 26);
  }
  return label;
}

function sheetTitleFromRange(range: string): string {
  const separator = range.lastIndexOf("!");
  if (separator < 0) return "";
  const title = range.slice(0, separator);
  return title.startsWith("'") && title.endsWith("'")
    ? title.slice(1, -1).split("''").join("'")
    : title;
}

function sheetRangeOrigin(range: string): { row: number; column: number } {
  const a1 = range.slice(range.lastIndexOf("!") + 1).split(":")[0].split("$").join("");
  const match = /^([A-Za-z]+)(\d+)$/.exec(a1);
  if (!match) return { row: 1, column: 0 };
  let column = 0;
  for (const character of match[1].toUpperCase()) {
    column = column * 26 + character.charCodeAt(0) - 64;
  }
  return { row: Number(match[2]), column: column - 1 };
}

function sheetColumnWidths(
  preview: Extract<GoogleFilePreview, { kind: "sheet" }>,
  width: number,
): number[] {
  return Array.from({ length: width }, (_, column) => {
    const longest = preview.values.reduce(
      (length, row, rowIndex) => Math.max(
        length,
        displayCell(row[column]).length,
        displayCell(preview.formulas[rowIndex]?.[column]).length,
      ),
      0,
    );
    return Math.max(96, Math.min(280, longest * 7 + 22));
  });
}

function displayCell(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return JSON.stringify(value);
}

function isRecord(value: unknown): value is Record<string, any> {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}
