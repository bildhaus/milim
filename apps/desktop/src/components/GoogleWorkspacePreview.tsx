import {
  Fragment,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ClipboardEvent,
  type CSSProperties,
  type FocusEvent,
  type FormEvent,
  type KeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

import {
  chooseGoogleWorkspaceFiles,
  editGoogleDoc,
  editGoogleSheet,
  editGoogleSlides,
  getGoogleFileContent,
  getGoogleFilePreview,
  getGoogleWorkspaceStatus,
  openExternalUrl,
  type GoogleFilePreview,
  type GoogleFileSummary,
  type GoogleSlidesEditOperation,
  type GoogleTextAlignment,
  type GoogleWorkspaceStatus,
  type PreviewSurfaceTarget,
} from "../api";
import {
  applyGoogleSheetDimension,
  applyGoogleSheetValues,
  createGoogleSaveQueue,
  googleDocEditableParagraph,
  googleDocEditableRegions,
  googleDocSelectionRange,
  googleDocTextReplacement,
  googleSheetCellRange,
  googleWorkspaceFileUrl,
  parseGoogleSheetClipboard,
  type GoogleDocEditableParagraph,
  type GoogleDocEditableRegion,
  type GoogleSaveQueueState,
} from "../lib/googleWorkspace";
import { useContextMenu } from "./ContextMenu";
import {
  AlignCenter,
  AlignJustify,
  AlignLeft,
  AlignRight,
  ArrowLeft,
  ArrowRight,
  ChevronDown,
  Copy,
  Eraser,
  ExternalLink,
  FileText,
  Folder,
  ListBullets,
  ListNumbers,
  MoreHorizontal,
  Plus,
  Refresh,
  Search,
  Sidebar,
  Trash,
} from "./icons";

export function GoogleWorkspacePreview({
  fileId,
  fallbackUrl,
  active,
  onMetadata,
  onSurfaceChange,
  onOpenFile,
  onOpenFileInNewTab,
}: {
  fileId: string;
  fallbackUrl: string;
  active: boolean;
  onMetadata?: (title: string, faviconUrl?: string) => void;
  onSurfaceChange?: (surface: PreviewSurfaceTarget | null) => void;
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

  useEffect(() => {
    if (!active) {
      onSurfaceChange?.(null);
      return;
    }
    onSurfaceChange?.({
      kind: "google_workspace",
      title: preview?.file.name ?? fallbackUrl,
      url: fallbackUrl,
      message: error,
      native: false,
      status: loading ? "loading" : error ? "error" : preview ? "ready" : "not_inspectable",
      capabilities: [],
    });
    return () => onSurfaceChange?.(null);
  }, [active, error, fallbackUrl, loading, onSurfaceChange, preview?.file.name]);

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
            <small>{googleFileKindDetail(preview.file)}</small>
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
          onSaved={refresh}
        />
      ) : preview.kind === "document" ? (
        <DocumentPreview
          fileId={preview.file.id}
          canEdit={preview.file.capabilities.can_edit}
          document={preview.document}
          fallbackText={preview.text}
          onSaved={refresh}
        />
      ) : preview.kind === "presentation" ? (
        <SlidesPreview
          fileId={preview.file.id}
          slides={preview.slides}
          pageAspectRatio={preview.pageAspectRatio ?? 16 / 9}
          pageWidth={preview.pageWidth ?? 720}
          pageHeight={preview.pageHeight ?? 405}
          active={active}
          canEdit={preview.file.capabilities.can_edit}
          onSaved={refresh}
        />
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

export function googleSlideThumbnailRequestKey(fileId: string, slideId: string, generation: number) {
  return `${fileId}\0${slideId}\0${generation}`;
}

function useGoogleSaveQueue(onDrained: () => void) {
  const onDrainedRef = useRef(onDrained);
  const [state, setState] = useState<GoogleSaveQueueState>({
    status: "idle",
    pending: 0,
    error: null,
  });
  const queueRef = useRef<ReturnType<typeof createGoogleSaveQueue> | null>(null);
  onDrainedRef.current = onDrained;
  queueRef.current ??= createGoogleSaveQueue(setState, () => onDrainedRef.current());
  return { ...state, enqueue: queueRef.current.enqueue, retry: queueRef.current.retry };
}

function GoogleAutosaveStatus({
  queue,
  dirty = false,
}: {
  queue: ReturnType<typeof useGoogleSaveQueue>;
  dirty?: boolean;
}) {
  if (queue.status === "error") {
    return (
      <span className="google-autosave-error" role="alert">
        Couldn’t save: {queue.error}
        <button type="button" onClick={queue.retry}>Retry</button>
      </span>
    );
  }
  if (dirty || queue.status === "saving") {
    return <small className="google-autosave-status" aria-live="polite">Saving...</small>;
  }
  return queue.status === "saved"
    ? <small className="google-autosave-status" aria-live="polite">Saved</small>
    : null;
}

export function SheetPreview({
  preview,
  range,
  setRange,
  loadRange,
  submitRange,
  onSaved,
}: {
  preview: Extract<GoogleFilePreview, { kind: "sheet" }>;
  range: string;
  setRange: (value: string) => void;
  loadRange: (value: string) => void;
  submitRange: (event: FormEvent) => void;
  onSaved: () => void;
}) {
  const sheets = preview.sheets
    .map((sheet) => {
      const properties = isRecord(sheet) && isRecord(sheet.properties) ? sheet.properties : null;
      return properties && typeof properties.title === "string"
        ? {
            title: properties.title,
            id: typeof properties.sheetId === "number" ? properties.sheetId : null,
          }
        : null;
    })
    .filter((sheet): sheet is { title: string; id: number | null } => Boolean(sheet));
  const saveQueue = useGoogleSaveQueue(onSaved);
  const [sheetGrid, setSheetGrid] = useState(() => ({
    values: preview.values,
    formulas: preview.formulas,
  }));
  const sheetSourceRef = useRef(`${preview.file.id}\0${preview.range}`);
  const width = Math.max(1, ...sheetGrid.values.map((row) => row.length), ...sheetGrid.formulas.map((row) => row.length));
  const gridRef = useRef<HTMLDivElement>(null);
  const editSourceRef = useRef<"cell" | "formula">("cell");
  const [selectedCell, setSelectedCell] = useState<[number, number]>([0, 0]);
  const [editingCell, setEditingCell] = useState<[number, number] | null>(null);
  const [columnWidths, setColumnWidths] = useState(() => sheetColumnWidths(preview, width));
  const [wrapCells, setWrapCells] = useState(false);
  const [search, setSearch] = useState("");
  const [zoom, setZoom] = useState(100);
  const [editValue, setEditValue] = useState("");
  const [savedEditValue, setSavedEditValue] = useState("");
  const [editError, setEditError] = useState<string | null>(null);
  const { openMenuAt } = useContextMenu();
  const activeSheet = sheetTitleFromRange(preview.range);
  const activeSheetId = sheets.find((sheet) => sheet.title === activeSheet)?.id ?? null;
  const rangeOrigin = sheetRangeOrigin(preview.range);
  const [selectedRow, selectedColumn] = selectedCell;
  const activeValue = displayCell(sheetGrid.values[selectedRow]?.[selectedColumn]);
  const activeFormula = displayCell(sheetGrid.formulas[selectedRow]?.[selectedColumn]);
  const activeContent = activeFormula.startsWith("=") ? activeFormula : activeValue;
  const activeAddress = `${columnLabel(rangeOrigin.column + selectedColumn)}${rangeOrigin.row + selectedRow}`;
  const matches = useMemo(() => {
    const needle = search.trim().toLocaleLowerCase();
    if (!needle) return [] as Array<[number, number]>;
    const found: Array<[number, number]> = [];
    sheetGrid.values.forEach((row, rowIndex) => {
      for (let columnIndex = 0; columnIndex < width; columnIndex += 1) {
        const value = `${displayCell(row[columnIndex])}\n${displayCell(sheetGrid.formulas[rowIndex]?.[columnIndex])}`;
        if (value.toLocaleLowerCase().includes(needle)) found.push([rowIndex, columnIndex]);
      }
    });
    return found;
  }, [search, sheetGrid.formulas, sheetGrid.values, width]);
  const matchKeys = useMemo(
    () => new Set(matches.map(([row, column]) => `${row}:${column}`)),
    [matches],
  );

  useEffect(() => {
    const source = `${preview.file.id}\0${preview.range}`;
    if (sheetSourceRef.current !== source || saveQueue.pending === 0) {
      sheetSourceRef.current = source;
      setSheetGrid({ values: preview.values, formulas: preview.formulas });
    }
  }, [preview.file.id, preview.formulas, preview.range, preview.values]);

  useEffect(() => {
    setSelectedCell([0, 0]);
    setColumnWidths(sheetColumnWidths(preview, width));
  }, [preview.range, width]);

  useEffect(() => {
    if (editingCell && editValue !== savedEditValue) return;
    setEditValue(activeContent);
    setSavedEditValue(activeContent);
    setEditError(null);
  }, [activeAddress, activeContent, editingCell]);

  useEffect(() => {
    gridRef.current
      ?.querySelector<HTMLElement>(`[data-sheet-cell="${selectedRow}:${selectedColumn}"]`)
      ?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [selectedColumn, selectedRow]);

  function selectCell(row: number, column: number) {
    setSelectedCell([
      Math.max(0, Math.min(sheetGrid.values.length - 1, row)),
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
    if ((event.key === "Enter" || event.key === "F2") && preview.file.capabilities.can_edit) {
      event.preventDefault();
      startCellEdit(selectedRow, selectedColumn);
      return;
    }
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

  function applyEdit(operations: Parameters<typeof editGoogleSheet>[1]) {
    setEditError(null);
    saveQueue.enqueue(() => editGoogleSheet(preview.file.id, operations));
  }

  function startCellEdit(row: number, column: number, source: "cell" | "formula" = "cell") {
    if (!preview.file.capabilities.can_edit) return;
    editSourceRef.current = source;
    if (editingCell?.[0] === row && editingCell[1] === column) return;
    queueCellSave();
    const value = displayCell(sheetGrid.values[row]?.[column]);
    const formula = displayCell(sheetGrid.formulas[row]?.[column]);
    const content = formula.startsWith("=") ? formula : value;
    setSelectedCell([row, column]);
    setEditingCell([row, column]);
    setEditValue(content);
    setSavedEditValue(content);
    setEditError(null);
  }

  const editingAddress = editingCell
    ? `${columnLabel(rangeOrigin.column + editingCell[1])}${rangeOrigin.row + editingCell[0]}`
    : "";
  const cellDirty = Boolean(editingCell) && editValue !== savedEditValue;

  function queueCellSave() {
    if (!editingCell || editValue === savedEditValue) return;
    const [row, column] = editingCell;
    const address = `${columnLabel(rangeOrigin.column + column)}${rangeOrigin.row + row}`;
    const snapshot = editValue;
    setSavedEditValue(snapshot);
    setSheetGrid((current) => applyGoogleSheetValues(current, row, column, [[snapshot]]));
    applyEdit([{
      action: "set_values",
      range: googleSheetCellRange(activeSheet, address),
      values: [[snapshot]],
      input_option: "USER_ENTERED",
    }]);
  }

  useEffect(() => {
    if (!cellDirty) return;
    const timeout = window.setTimeout(queueCellSave, 800);
    return () => window.clearTimeout(timeout);
  }, [cellDirty, editValue, editingAddress]);

  function finishCellEdit(moveDown = false, focusGrid = false) {
    if (!editingCell) return;
    const [row, column] = editingCell;
    queueCellSave();
    setEditingCell(null);
    if (moveDown) selectCell(row + 1, column);
    if (focusGrid) gridRef.current?.focus();
  }

  function cancelCellEdit() {
    setEditValue(savedEditValue);
    setEditingCell(null);
    gridRef.current?.focus();
  }

  function handleEditorBlur(event: FocusEvent<HTMLInputElement>) {
    if (event.relatedTarget instanceof HTMLElement && event.relatedTarget.dataset.sheetEditor === "true") return;
    void finishCellEdit();
  }

  function handleEditorKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      cancelCellEdit();
    } else if (event.key === "Enter") {
      event.preventDefault();
      void finishCellEdit(true, true);
    }
  }

  function pasteCells(event: ClipboardEvent<HTMLDivElement>) {
    if (!preview.file.capabilities.can_edit) return;
    const values = parseGoogleSheetClipboard(event.clipboardData.getData("text/plain"));
    const cells = values.reduce((count, row) => count + row.length, 0);
    if (!cells) return;
    event.preventDefault();
    if (cells > 5_000) {
      setEditError("Paste is limited to 5,000 cells.");
      return;
    }
    setSheetGrid((current) => applyGoogleSheetValues(current, selectedRow, selectedColumn, values));
    applyEdit([{
      action: "set_values",
      range: googleSheetCellRange(activeSheet, activeAddress),
      values,
      input_option: "USER_ENTERED",
    }]);
  }

  function editDimension(
    action: "insert_rows" | "delete_rows" | "insert_columns" | "delete_columns",
  ) {
    if (activeSheetId === null) return;
    const deleting = action.startsWith("delete");
    const row = action.endsWith("rows");
    const selectedIndex = row
      ? rangeOrigin.row - 1 + selectedRow
      : rangeOrigin.column + selectedColumn;
    if (deleting && !window.confirm(`Delete ${row ? "row" : "column"} ${row ? rangeOrigin.row + selectedRow : columnLabel(rangeOrigin.column + selectedColumn)}?`)) {
      return;
    }
    const start = deleting ? selectedIndex : selectedIndex + 1;
    const localIndex = row
      ? (deleting ? selectedRow : selectedRow + 1)
      : (deleting ? selectedColumn : selectedColumn + 1);
    setSheetGrid((current) => applyGoogleSheetDimension(current, action, localIndex));
    applyEdit([{
      action,
      sheet_id: activeSheetId,
      start,
      end: start + 1,
    }]);
  }

  function openDimensionMenu(event: ReactMouseEvent<HTMLButtonElement>) {
    const trigger = event.currentTarget;
    const rect = trigger.getBoundingClientRect();
    const disabled = activeSheetId === null || !preview.file.capabilities.can_edit;
    openMenuAt({ x: rect.left, y: rect.bottom + 4 }, [
      {
        id: "insert-sheet-row",
        label: "Insert row below",
        detail: `${rangeOrigin.row + selectedRow}`,
        icon: <Plus size={13} />,
        disabled,
        action: () => editDimension("insert_rows"),
      },
      {
        id: "delete-sheet-row",
        label: `Delete row ${rangeOrigin.row + selectedRow}`,
        icon: <Trash size={13} />,
        disabled,
        danger: true,
        action: () => editDimension("delete_rows"),
      },
      {
        id: "insert-sheet-column",
        label: "Insert column right",
        detail: columnLabel(rangeOrigin.column + selectedColumn),
        icon: <Plus size={13} />,
        disabled,
        separatorBefore: true,
        action: () => editDimension("insert_columns"),
      },
      {
        id: "delete-sheet-column",
        label: `Delete column ${columnLabel(rangeOrigin.column + selectedColumn)}`,
        icon: <Trash size={13} />,
        disabled,
        danger: true,
        action: () => editDimension("delete_columns"),
      },
    ], "Row and column actions", trigger);
  }

  return (
    <div className="google-sheet-preview">
      <form className="google-sheet-toolbar" aria-label="Sheet controls" onSubmit={submitRange}>
        <label className="google-sheet-range">
          <input
            value={range}
            title="Google Sheets range"
            aria-label="Google Sheets range"
            onChange={(event) => setRange(event.currentTarget.value)}
          />
          <button type="submit" title="Load range" aria-label="Load range">
            <ArrowRight size={13} />
          </button>
        </label>
        <label className="google-sheet-search">
          <Search size={13} aria-hidden="true" />
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
        <button
          className="google-sheet-actions-trigger"
          type="button"
          title="Row and column actions"
          aria-label="Row and column actions"
          aria-haspopup="menu"
          disabled={activeSheetId === null || !preview.file.capabilities.can_edit}
          onClick={openDimensionMenu}
        >
          <MoreHorizontal size={14} />
        </button>
      </form>
      <div className="google-sheet-formula-bar" aria-live="polite">
        <output aria-label="Active cell">{activeAddress}</output>
        <span className="google-sheet-formula-symbol" aria-hidden="true">fx</span>
        <input
          className="google-sheet-formula-input"
          data-sheet-editor="true"
          aria-label={`Edit active cell ${activeAddress}`}
          readOnly={!preview.file.capabilities.can_edit}
          value={editingCell ? editValue : activeContent}
          onFocus={() => {
            if (preview.file.capabilities.can_edit) startCellEdit(selectedRow, selectedColumn, "formula");
          }}
          onChange={(event) => setEditValue(event.currentTarget.value)}
          onBlur={handleEditorBlur}
          onKeyDown={handleEditorKeyDown}
        />
        {preview.file.capabilities.can_edit
          ? <GoogleAutosaveStatus queue={saveQueue} dirty={cellDirty} />
          : <small>View only</small>}
      </div>
      {editError ? <div className="google-sheet-edit-error" role="alert">{editError}</div> : null}
      <div
        className="google-sheet-grid-wrap"
        ref={gridRef}
        tabIndex={0}
        aria-label="Spreadsheet grid. Use arrow keys to move between cells."
        onKeyDown={handleGridKeyDown}
        onPaste={pasteCells}
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
            {sheetGrid.values.length ? sheetGrid.values.map((row, rowIndex) => (
              <tr key={rowIndex}>
                <th>{rangeOrigin.row + rowIndex}</th>
                {Array.from({ length: width }, (_, columnIndex) => {
                  const formula = displayCell(sheetGrid.formulas[rowIndex]?.[columnIndex]);
                  const active = rowIndex === selectedRow && columnIndex === selectedColumn;
                  const editing = editingCell?.[0] === rowIndex && editingCell[1] === columnIndex;
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
                      onDoubleClick={() => {
                        startCellEdit(rowIndex, columnIndex);
                      }}
                    >
                      {editing ? (
                        <input
                          className="google-sheet-cell-editor"
                          data-sheet-editor="true"
                          aria-label={`Edit cell ${editingAddress}`}
                          value={editValue}
                          autoFocus={editSourceRef.current === "cell"}
                          onClick={(event) => event.stopPropagation()}
                          onDoubleClick={(event) => event.stopPropagation()}
                          onChange={(event) => setEditValue(event.currentTarget.value)}
                          onBlur={handleEditorBlur}
                          onKeyDown={handleEditorKeyDown}
                        />
                      ) : (
                        <>
                          <span>{displayCell(row[columnIndex])}</span>
                          {formula.startsWith("=") ? <code>{formula}</code> : null}
                        </>
                      )}
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
      <div className="google-sheet-tabs" aria-label="Worksheets">
        {sheets.map(({ title }) => (
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
    </div>
  );
}

type GoogleTextFormat = {
  bold: boolean;
  italic: boolean;
  underline: boolean;
  fontSize: number;
  fontFamily?: string;
  color: string;
  alignment: GoogleTextAlignment;
};

function googleTextFormatCss(format: GoogleTextFormat | null): CSSProperties {
  return format ? {
    color: format.color,
    fontSize: `${format.fontSize}px`,
    fontFamily: format.fontFamily,
    fontWeight: format.bold ? 700 : undefined,
    fontStyle: format.italic ? "italic" : undefined,
    textDecoration: format.underline ? "underline" : undefined,
    textAlign: format.alignment === "JUSTIFIED"
      ? "justify"
      : format.alignment.toLocaleLowerCase() as CSSProperties["textAlign"],
  } : {};
}

type GoogleTextFormatChange =
  | { field: "bold" | "italic" | "underline"; value: boolean }
  | { field: "fontSize"; value: number }
  | { field: "color"; value: string }
  | { field: "alignment"; value: GoogleTextAlignment };

export function GoogleTextFormatToolbar({
  format,
  disabled,
  className = "",
  onChange,
}: {
  format: GoogleTextFormat;
  disabled?: boolean;
  className?: string;
  onChange: (change: GoogleTextFormatChange) => void;
}) {
  const AlignmentIcon = format.alignment === "CENTER"
    ? AlignCenter
    : format.alignment === "END"
      ? AlignRight
      : format.alignment === "JUSTIFIED"
        ? AlignJustify
        : AlignLeft;
  return (
    <span
      className={`google-text-format-toolbar ${className}`.trim()}
      role="toolbar"
      aria-label="Text formatting"
      data-google-format-toolbar="true"
    >
      <span className="google-toolbar-group">
        {(["bold", "italic", "underline"] as const).map((field) => (
          <button
            type="button"
            key={field}
            className={format[field] ? "active" : ""}
            title={field[0].toUpperCase() + field.slice(1)}
            aria-label={field[0].toUpperCase() + field.slice(1)}
            aria-pressed={format[field]}
            disabled={disabled}
            onClick={() => onChange({ field, value: !format[field] })}
          >
            {field === "bold" ? <strong>B</strong> : field === "italic" ? <em>I</em> : <u>U</u>}
          </button>
        ))}
      </span>
      <span className="google-toolbar-divider" aria-hidden="true" />
      <label className="google-toolbar-value-select" title="Font size">
        <span aria-hidden="true">{format.fontSize}</span>
        <ChevronDown size={10} aria-hidden="true" />
        <select
          aria-label="Font size"
          value={format.fontSize}
          disabled={disabled}
          onChange={(event) => onChange({ field: "fontSize", value: Number(event.currentTarget.value) })}
        >
          {[8, 10, 11, 12, 14, 18, 24, 32, 48, 72].map((value) => (
            <option value={value} key={value}>{value}</option>
          ))}
        </select>
      </label>
      <label className="google-toolbar-icon-select" title="Text alignment">
        <AlignmentIcon size={15} aria-hidden="true" />
        <ChevronDown size={9} aria-hidden="true" />
        <select
          aria-label="Text alignment"
          value={format.alignment}
          disabled={disabled}
          onChange={(event) => onChange({
            field: "alignment",
            value: event.currentTarget.value as GoogleTextAlignment,
          })}
        >
          <option value="START">Align start</option>
          <option value="CENTER">Center</option>
          <option value="END">Align end</option>
          <option value="JUSTIFIED">Justify</option>
        </select>
      </label>
      <label className="google-text-color-control" title="Text color">
        <span
          className="google-text-color-glyph"
          style={{ "--google-text-color": format.color } as CSSProperties}
          aria-hidden="true"
        >
          A
        </span>
        <input
          type="color"
          aria-label="Text color"
          value={format.color}
          disabled={disabled}
          onChange={(event) => onChange({ field: "color", value: event.currentTarget.value })}
        />
      </label>
    </span>
  );
}

type GoogleDocParagraphFormat = GoogleTextFormat & {
  namedStyleType: string;
  listKind: "ordered" | "unordered" | null;
};

type GoogleDocEditorState = GoogleDocEditableParagraph & GoogleDocParagraphFormat & {
  regionBlockIndex: number;
  paragraphStart: number;
  paragraphEnd: number;
};

type GoogleDocNamedStyleMap = Record<string, {
  paragraphStyle: Record<string, unknown>;
  textStyle: Record<string, unknown>;
}>;

export function DocumentPreview({
  fileId,
  canEdit,
  document,
  fallbackText,
  onSaved,
}: {
  fileId: string;
  canEdit: boolean;
  document: Record<string, unknown>;
  fallbackText: string;
  onSaved: () => void;
}) {
  const [query, setQuery] = useState("");
  const [matchCount, setMatchCount] = useState(0);
  const [activeMatch, setActiveMatch] = useState(0);
  const [zoom, setZoom] = useState<"fit" | number>("fit");
  const [fitScale, setFitScale] = useState(1);
  const [outlineOpen, setOutlineOpen] = useState(false);
  const [editing, setEditing] = useState<GoogleDocEditorState | null>(null);
  const [editorText, setEditorText] = useState("");
  const [savedEditorText, setSavedEditorText] = useState("");
  const [editorSelection, setEditorSelection] = useState({ start: 0, end: 0 });
  const [hasTextSelection, setHasTextSelection] = useState(false);
  const dirtyDocumentRef = useRef(false);
  const editorTextRef = useRef("");
  const savedEditorTextRef = useRef("");
  const saveQueue = useGoogleSaveQueue(() => {
    if (!dirtyDocumentRef.current) onSaved();
  });
  const contentRef = useRef<HTMLDivElement | null>(null);
  const editorElementRef = useRef<HTMLDivElement | null>(null);
  const savedEditorHtmlRef = useRef("");
  const cancelEditorBlurRef = useRef(false);
  const selectedRangeRef = useRef<Range | null>(null);
  const body = isRecord(document.body) && Array.isArray(document.body.content)
    ? document.body.content
    : [];
  const inlineObjects = isRecord(document.inlineObjects)
    ? document.inlineObjects
    : {};
  const lists = isRecord(document.lists) ? document.lists : {};
  const namedStyles = useMemo(() => googleDocNamedStyleMap(document), [document]);
  const outline = useMemo(() => googleDocOutline(body), [body]);
  const editableRegions = useMemo(() => googleDocEditableRegions(body), [body]);
  const editableRegionByBlock = useMemo(() => {
    const regions = new Map<number, { region: GoogleDocEditableRegion; first: boolean }>();
    editableRegions.forEach((region) => {
      region.blockIndexes.forEach((blockIndex, index) => {
        regions.set(blockIndex, { region, first: index === 0 });
      });
    });
    return regions;
  }, [editableRegions]);
  const normalizedQuery = query.trim();

  useEffect(() => {
    const canvas = contentRef.current;
    if (!canvas) return;
    const updateScale = () => {
      const style = window.getComputedStyle(canvas);
      setFitScale(googleDocFitScale(
        canvas.clientWidth,
        Number.parseFloat(style.paddingLeft) || 0,
        Number.parseFloat(style.paddingRight) || 0,
      ));
    };
    const observer = new ResizeObserver(updateScale);
    updateScale();
    observer.observe(canvas);
    return () => observer.disconnect();
  }, []);

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

  function paragraphFormat(blockIndex: number): GoogleDocParagraphFormat {
    const value = body[blockIndex];
    if (!isRecord(value) || !isRecord(value.paragraph)) {
      return {
        namedStyleType: "NORMAL_TEXT",
        listKind: null,
        bold: false,
        italic: false,
        underline: false,
        fontSize: 11,
        color: "#000000",
        alignment: "START",
      };
    }
    const directStyle = isRecord(value.paragraph.paragraphStyle) ? value.paragraph.paragraphStyle : {};
    const namedStyle = typeof directStyle.namedStyleType === "string" ? directStyle.namedStyleType : "NORMAL_TEXT";
    const inherited = namedStyles[namedStyle] ?? namedStyles.NORMAL_TEXT;
    const style = { ...(namedStyles.NORMAL_TEXT?.paragraphStyle ?? {}), ...(inherited?.paragraphStyle ?? {}), ...directStyle };
    const textStyle = { ...(namedStyles.NORMAL_TEXT?.textStyle ?? {}), ...(inherited?.textStyle ?? {}) };
    return googleDocParagraphFormat(value.paragraph, style, textStyle, namedStyle, lists);
  }

  function editRegion(
    region: GoogleDocEditableRegion,
    element: HTMLDivElement,
  ) {
    if (!canEdit) return;
    if (editorElementRef.current === element && editing?.regionBlockIndex === region.blockIndexes[0]) return;
    if (editorElementRef.current !== element) {
      queueDocumentText();
      editorElementRef.current = element;
    }
    const firstParagraph = googleDocEditableParagraph(body[region.blockIndexes[0]]) ?? region;
    setEditing({
      ...region,
      regionBlockIndex: region.blockIndexes[0],
      paragraphStart: firstParagraph.start,
      paragraphEnd: firstParagraph.end,
      ...paragraphFormat(region.blockIndexes[0]),
    });
    setEditorText(region.text);
    setSavedEditorText(region.text);
    editorTextRef.current = region.text;
    savedEditorTextRef.current = region.text;
    dirtyDocumentRef.current = false;
    savedEditorHtmlRef.current = element.innerHTML;
    setEditorSelection({ start: region.start, end: region.start });
    selectedRangeRef.current = null;
    setHasTextSelection(false);
  }

  function updateEditorText(element: HTMLDivElement) {
    const next = Array.from(element.childNodes, (node) => node.textContent ?? "").join("\n").replace(/\r\n?/g, "\n");
    editorTextRef.current = next;
    dirtyDocumentRef.current = next !== savedEditorTextRef.current;
    setEditorText(next);
  }

  function updateEditorSelection(element: HTMLDivElement, region: GoogleDocEditableRegion) {
    const selection = window.getSelection();
    if (!selection || selection.rangeCount === 0) return;
    const range = selection.getRangeAt(0);
    if (!element.contains(range.startContainer) || !element.contains(range.endContainer)) return;
    const surfaceOffset = (container: Node, offset: number) => {
      const children = Array.from(element.childNodes);
      if (container === element) {
        const before = children.slice(0, offset);
        return before.reduce((total, node) => total + (node.textContent?.length ?? 0), 0)
          + Math.min(offset, Math.max(0, children.length - 1));
      }
      let total = 0;
      for (const child of children) {
        if (child === container || child.contains(container)) {
          const prefix = range.cloneRange();
          prefix.selectNodeContents(child);
          prefix.setEnd(container, offset);
          return total + prefix.toString().length;
        }
        total += (child.textContent?.length ?? 0) + 1;
      }
      return null;
    };
    const startOffset = surfaceOffset(range.startContainer, range.startOffset);
    const endOffset = surfaceOffset(range.endContainer, range.endOffset);
    if (startOffset === null || endOffset === null) return;
    const nextSelection = {
      start: region.start + startOffset,
      end: region.start + endOffset,
    };
    setEditorSelection(nextSelection);
    selectedRangeRef.current = range.cloneRange();
    setHasTextSelection(!range.collapsed);
    const paragraphNode = (range.startContainer instanceof Element
      ? range.startContainer
      : range.startContainer.parentElement)?.closest<HTMLElement>("[data-doc-block-index]");
    const blockIndex = Number(paragraphNode?.dataset.docBlockIndex);
    const paragraph = Number.isInteger(blockIndex) ? googleDocEditableParagraph(body[blockIndex]) : null;
    if (paragraph) {
      setEditing((current) => current ? {
        ...current,
        paragraphStart: paragraph.start,
        paragraphEnd: paragraph.end,
        ...paragraphFormat(blockIndex),
      } : current);
    }
  }

  const docDirty = Boolean(editing) && editorText !== savedEditorText;

  function queueDocumentText() {
    const snapshot = editorTextRef.current;
    const saved = savedEditorTextRef.current;
    if (!editing || snapshot === saved) return;
    const replacement = googleDocTextReplacement(editing.start, saved, snapshot);
    if (!replacement) return;
    const operations: Parameters<typeof editGoogleDoc>[1] = [];
    if (replacement.end > replacement.start) {
      operations.push({ action: "delete_range", start: replacement.start, end: replacement.end });
    }
    if (replacement.text) {
      operations.push({ action: "insert_text", index: replacement.start, text: replacement.text });
    }
    setSavedEditorText(snapshot);
    savedEditorTextRef.current = snapshot;
    dirtyDocumentRef.current = false;
    setEditing((current) => current ? { ...current, end: current.start + snapshot.length, text: snapshot } : current);
    if (editorElementRef.current) savedEditorHtmlRef.current = editorElementRef.current.innerHTML;
    saveQueue.enqueue(() => editGoogleDoc(fileId, operations));
  }

  useEffect(() => {
    if (!docDirty) return;
    const timeout = window.setTimeout(queueDocumentText, 800);
    return () => window.clearTimeout(timeout);
  }, [docDirty, editorText, editing?.regionBlockIndex]);

  function applyParagraphFormat(
    operations: Parameters<typeof editGoogleDoc>[1],
    patch: Partial<GoogleDocParagraphFormat>,
  ) {
    if (!editing) return;
    queueDocumentText();
    const nextEditing = { ...editing, ...patch };
    setEditing(nextEditing);
    if (editorElementRef.current) savedEditorHtmlRef.current = editorElementRef.current.innerHTML;
    saveQueue.enqueue(() => editGoogleDoc(fileId, operations));
  }

  function editorBlur(event: FocusEvent<HTMLElement>) {
    if (cancelEditorBlurRef.current) {
      cancelEditorBlurRef.current = false;
      return;
    }
    if (
      event.relatedTarget instanceof Node
      && (event.currentTarget.contains(event.relatedTarget)
        || event.relatedTarget instanceof HTMLElement && event.relatedTarget.closest("[data-google-format-toolbar]"))
    ) return;
    queueDocumentText();
    setEditing(null);
    editorElementRef.current = null;
    selectedRangeRef.current = null;
    setHasTextSelection(false);
  }

  const editorRange = editing
    ? editorSelection.end > editorSelection.start
      ? editorSelection
      : { start: editing.paragraphStart, end: editing.paragraphEnd }
    : null;

  function restoreDocumentSelection() {
    const range = selectedRangeRef.current;
    if (!range) return;
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
  }

  function selectedDocumentBlocks() {
    const editor = editorElementRef.current;
    const range = selectedRangeRef.current;
    if (!editor || !range) return [];
    return Array.from(editor.children).filter((element): element is HTMLElement => (
      element instanceof HTMLElement && range.intersectsNode(element)
    ));
  }

  function applyNativeTextFormat(change: GoogleTextFormatChange) {
    restoreDocumentSelection();
    if (change.field === "fontSize") {
      window.document.execCommand("fontSize", false, "7");
      editorElementRef.current?.querySelectorAll<HTMLElement>('font[size="7"]').forEach((element) => {
        element.removeAttribute("size");
        element.style.fontSize = `${change.value}pt`;
      });
    } else if (change.field === "color") {
      window.document.execCommand("foreColor", false, change.value);
    } else if (change.field === "alignment") {
      const command = change.value === "CENTER"
        ? "justifyCenter"
        : change.value === "END"
          ? "justifyRight"
          : change.value === "JUSTIFIED"
            ? "justifyFull"
            : "justifyLeft";
      window.document.execCommand(command);
    } else {
      window.document.execCommand(change.field);
    }
    if (editorElementRef.current) savedEditorHtmlRef.current = editorElementRef.current.innerHTML;
  }

  function applyDocumentTextFormat(change: GoogleTextFormatChange) {
    if (!editorRange) return;
    applyNativeTextFormat(change);
    if (change.field === "alignment") {
      void applyParagraphFormat(
        [{ action: "set_paragraph_style", ...editorRange, alignment: change.value }],
        { alignment: change.value },
      );
    } else if (change.field === "fontSize") {
      void applyParagraphFormat(
        [{ action: "set_text_style", ...editorRange, font_size: change.value }],
        { fontSize: change.value },
      );
    } else if (change.field === "color") {
      void applyParagraphFormat(
        [{ action: "set_text_style", ...editorRange, foreground_color: change.value }],
        { color: change.value },
      );
    } else {
      void applyParagraphFormat(
        [{ action: "set_text_style", ...editorRange, [change.field]: change.value }],
        { [change.field]: change.value },
      );
    }
  }

  function applyDocumentNamedStyle(namedStyleType: string) {
    if (!editorRange) return;
    restoreDocumentSelection();
    const tag = namedStyleType === "TITLE"
      ? "h1"
      : namedStyleType.startsWith("HEADING_")
        ? `h${Math.min(6, Number(namedStyleType.slice(-1)) + 1)}`
        : "p";
    window.document.execCommand("formatBlock", false, tag);
    applyParagraphFormat(
      [{ action: "set_paragraph_style", ...editorRange, named_style_type: namedStyleType }],
      { namedStyleType },
    );
  }

  function toggleDocumentList(kind: "unordered" | "ordered") {
    if (!editing || !editorRange) return;
    const active = editing.listKind === kind;
    selectedDocumentBlocks().forEach((element) => {
      element.classList.toggle("google-doc-list-item", !active);
      if (active) {
        element.removeAttribute("data-list-kind");
        element.style.removeProperty("padding-inline-start");
      } else {
        element.dataset.listKind = kind;
        element.style.paddingInlineStart = "18px";
      }
    });
    applyParagraphFormat(
      active
        ? [{ action: "delete_bullets", ...editorRange }]
        : [{
          action: "create_bullets",
          ...editorRange,
          bullet_preset: kind === "unordered"
            ? "BULLET_DISC_CIRCLE_SQUARE"
            : "NUMBERED_DECIMAL_ALPHA_ROMAN",
        }],
      { listKind: active ? null : kind },
    );
  }

  function clearDocumentFormatting() {
    if (!editing || !editorRange) return;
    const blocks = selectedDocumentBlocks();
    restoreDocumentSelection();
    window.document.execCommand("removeFormat");
    blocks.forEach((element) => {
      element.classList.remove("google-doc-list-item");
      element.removeAttribute("data-list-kind");
      element.removeAttribute("style");
    });
    const operations: Parameters<typeof editGoogleDoc>[1] = [
      { action: "clear_text_style", ...editorRange },
      { action: "set_paragraph_style", ...editorRange, named_style_type: "NORMAL_TEXT", alignment: "START" },
    ];
    if (editing.listKind) operations.push({ action: "delete_bullets", ...editorRange });
    applyParagraphFormat(operations, {
      namedStyleType: "NORMAL_TEXT",
      bold: false,
      italic: false,
      underline: false,
      fontSize: 11,
      color: "#000000",
      alignment: "START",
      listKind: null,
    });
  }

  return (
    <div className="google-doc-viewer">
      <div className="google-viewer-toolbar google-doc-unified-toolbar" role="toolbar" aria-label="Document controls">
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
        {canEdit ? (
          hasTextSelection && editing ? (
            <GoogleTextFormatToolbar
              format={editing}
              disabled={!editorRange}
              className="google-doc-contextual-tools"
              onChange={applyDocumentTextFormat}
            />
          ) : (
            <span
              className="google-text-format-toolbar google-doc-contextual-tools"
              role="toolbar"
              aria-label="Paragraph formatting"
              data-google-format-toolbar="true"
            >
              <label className="google-paragraph-style-control" title="Paragraph style">
                <select
                  aria-label="Paragraph style"
                  value={editing?.namedStyleType ?? "NORMAL_TEXT"}
                  disabled={!editorRange}
                  onChange={(event) => applyDocumentNamedStyle(event.currentTarget.value)}
                >
                  <option value="NORMAL_TEXT">Normal text</option>
                  <option value="TITLE">Title</option>
                  <option value="SUBTITLE">Subtitle</option>
                  {[1, 2, 3, 4, 5, 6].map((level) => (
                    <option value={`HEADING_${level}`} key={level}>Heading {level}</option>
                  ))}
                </select>
              </label>
              <span className="google-toolbar-divider" aria-hidden="true" />
              {(["unordered", "ordered"] as const).map((kind) => (
                <button
                  type="button"
                  key={kind}
                  className={editing?.listKind === kind ? "active" : ""}
                  aria-label={kind === "unordered" ? "Bulleted list" : "Numbered list"}
                  aria-pressed={editing?.listKind === kind}
                  disabled={!editorRange}
                  title={kind === "unordered" ? "Bulleted list" : "Numbered list"}
                  onClick={() => toggleDocumentList(kind)}
                >
                  {kind === "unordered"
                    ? <ListBullets size={15} aria-hidden="true" />
                    : <ListNumbers size={15} aria-hidden="true" />}
                </button>
              ))}
              <button
                type="button"
                aria-label="Clear formatting"
                disabled={!editorRange}
                title="Clear formatting"
                onClick={clearDocumentFormatting}
              >
                <Eraser size={15} aria-hidden="true" />
              </button>
            </span>
          )
        ) : null}
        <span className="google-doc-toolbar-status">
          <GoogleAutosaveStatus queue={saveQueue} dirty={docDirty} />
        </span>
        <span className="google-viewer-toolbar-spacer" />
        <button
          className={`preview-browser-action${outlineOpen ? " active" : ""}`}
          type="button"
          title={outlineOpen ? "Hide document outline" : "Show document outline"}
          aria-label={outlineOpen ? "Hide document outline" : "Show document outline"}
          aria-pressed={outlineOpen}
          disabled={!outline.length}
          onClick={() => setOutlineOpen((value) => !value)}
        >
          <Sidebar size={13} />
        </button>
        <label className="google-viewer-zoom">
          <span>Zoom</span>
          <select
            aria-label="Document zoom"
            value={zoom}
            onChange={(event) => setZoom(event.target.value === "fit" ? "fit" : Number(event.target.value))}
          >
            <option value="fit">Fit width</option>
            {[75, 90, 100, 110, 125, 150].map((value) => <option value={value} key={value}>{value}%</option>)}
          </select>
        </label>
      </div>
      <div className={`google-doc-body${outlineOpen && outline.length ? " with-outline" : ""}`}>
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
            className="google-doc-preview"
            style={{ "--doc-zoom": zoom === "fit" ? fitScale : zoom / 100 } as CSSProperties}
          >
            {body.length ? body.map((item, index) => {
              const editableRegion = canEdit ? editableRegionByBlock.get(index) : undefined;
              if (editableRegion && !editableRegion.first) return null;
              if (editableRegion) {
                const { region } = editableRegion;
                return (
                  <div
                    className="google-doc-editable-region"
                    contentEditable
                    suppressContentEditableWarning
                    role="textbox"
                    aria-label="Edit document text"
                    aria-multiline="true"
                    title="Click text to edit"
                    data-active={editing?.regionBlockIndex === region.blockIndexes[0] ? "true" : undefined}
                    onFocus={(event) => {
                      editRegion(region, event.currentTarget);
                      window.requestAnimationFrame(() => updateEditorSelection(event.currentTarget, region));
                    }}
                    onInput={(event) => updateEditorText(event.currentTarget)}
                    onSelect={(event) => updateEditorSelection(event.currentTarget, region)}
                    onKeyUp={(event) => updateEditorSelection(event.currentTarget, region)}
                    onPaste={(event) => {
                      event.preventDefault();
                      window.document.execCommand("insertText", false, event.clipboardData.getData("text/plain"));
                    }}
                    onBlur={editorBlur}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.preventDefault();
                        window.document.execCommand("insertText", false, "\n");
                      } else if (event.key === "Escape") {
                        event.preventDefault();
                        cancelEditorBlurRef.current = true;
                        event.currentTarget.innerHTML = savedEditorHtmlRef.current;
                        editorTextRef.current = savedEditorTextRef.current;
                        dirtyDocumentRef.current = false;
                        setEditorText(savedEditorTextRef.current);
                        setEditing(null);
                        editorElementRef.current = null;
                        selectedRangeRef.current = null;
                        setHasTextSelection(false);
                        event.currentTarget.blur();
                      }
                    }}
                    key={`editable-${region.start}`}
                  >
                    {region.blockIndexes.map((blockIndex) => (
                      <DocStructuralElement
                        value={body[blockIndex]}
                        inlineObjects={inlineObjects}
                        lists={lists}
                        namedStyles={namedStyles}
                        query={normalizedQuery}
                        blockIndex={blockIndex}
                        key={blockIndex}
                      />
                    ))}
                  </div>
                );
              }
              return (
                <DocStructuralElement
                  value={item}
                  inlineObjects={inlineObjects}
                  lists={lists}
                  namedStyles={namedStyles}
                  query={normalizedQuery}
                  blockIndex={index}
                  key={index}
                />
              );
            }) : (
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
  namedStyles,
  query,
  blockIndex,
}: {
  value: unknown;
  inlineObjects: Record<string, unknown>;
  lists: Record<string, unknown>;
  namedStyles: GoogleDocNamedStyleMap;
  query: string;
  blockIndex?: number;
}): ReactNode {
  if (!isRecord(value)) return null;
  if (isRecord(value.paragraph) && Array.isArray(value.paragraph.elements)) {
    const directStyle = isRecord(value.paragraph.paragraphStyle) ? value.paragraph.paragraphStyle : {};
    const namedStyle = typeof directStyle.namedStyleType === "string" ? directStyle.namedStyleType : "NORMAL_TEXT";
    const inherited = namedStyles[namedStyle] ?? namedStyles.NORMAL_TEXT;
    const style = { ...(namedStyles.NORMAL_TEXT?.paragraphStyle ?? {}), ...(inherited?.paragraphStyle ?? {}), ...directStyle };
    const textStyle = { ...(namedStyles.NORMAL_TEXT?.textStyle ?? {}), ...(inherited?.textStyle ?? {}) };
    const content = value.paragraph.elements.map((element, index) => (
      <DocParagraphElement
        value={element}
        inlineObjects={inlineObjects}
        inheritedStyle={textStyle}
        query={query}
        trimTrailingNewline={index === value.paragraph.elements.length - 1}
        key={index}
      />
    ));
    const paragraphStyle = googleDocParagraphStyle(style);
    const paragraphProps = blockIndex === undefined ? {} : {
      "data-doc-heading": blockIndex,
      "data-doc-block-index": blockIndex,
    };
    let paragraphNode: ReactNode;
    const visibleList = isRecord(value.paragraph.bullet) ? googleDocListKind(value.paragraph.bullet, lists) : null;
    if (namedStyle === "TITLE") {
      paragraphNode = <h1 style={paragraphStyle} {...paragraphProps}>{content}</h1>;
    } else if (namedStyle === "SUBTITLE") {
      paragraphNode = <p className="google-doc-subtitle" style={paragraphStyle} {...paragraphProps}>{content}</p>;
    } else if (namedStyle.startsWith("HEADING_")) {
      const level = Math.min(6, Math.max(2, Number(namedStyle.slice(-1)) + 1));
      if (level === 2) paragraphNode = <h2 style={paragraphStyle} {...paragraphProps}>{content}</h2>;
      else if (level === 3) paragraphNode = <h3 style={paragraphStyle} {...paragraphProps}>{content}</h3>;
      else if (level === 4) paragraphNode = <h4 style={paragraphStyle} {...paragraphProps}>{content}</h4>;
      else if (level === 5) paragraphNode = <h5 style={paragraphStyle} {...paragraphProps}>{content}</h5>;
      else paragraphNode = <h6 style={paragraphStyle} {...paragraphProps}>{content}</h6>;
    } else if (visibleList) {
      const bullet = isRecord(value.paragraph.bullet) ? value.paragraph.bullet : {};
      const nestingLevel = typeof bullet.nestingLevel === "number" ? bullet.nestingLevel : 0;
      paragraphNode = (
        <p
          className="google-doc-list-item"
          data-list-kind={visibleList}
          {...paragraphProps}
          style={{ ...paragraphStyle, paddingInlineStart: `${18 + nestingLevel * 18}px` }}
        >
          {content}
        </p>
      );
    } else {
      paragraphNode = <p style={paragraphStyle} {...paragraphProps}>{content}</p>;
    }
    return paragraphNode;
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
                      <DocStructuralElement value={item} inlineObjects={inlineObjects} lists={lists} namedStyles={namedStyles} query={query} key={index} />
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
  inheritedStyle,
  query,
  trimTrailingNewline = false,
}: {
  value: unknown;
  inlineObjects: Record<string, unknown>;
  inheritedStyle: Record<string, unknown>;
  query: string;
  trimTrailingNewline?: boolean;
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
  const directStyle = isRecord(value.textRun.textStyle) ? value.textRun.textStyle : {};
  const style = { ...inheritedStyle, ...directStyle };
  const textStyle = googleDocTextStyle(style);
  const content = trimTrailingNewline ? value.textRun.content.replace(/\n$/, "") : value.textRun.content;
  return (
    <span className={isRecord(style.link) ? "google-doc-link" : undefined} style={textStyle}>
      {highlightGoogleDocText(content, query)}
    </span>
  );
}

export type GoogleSlideEditableField = {
  id: string;
  label: string;
  text: string;
  kind: "shape" | "notes";
  styleRuns?: Array<{ start: number; end: number; style: Record<string, unknown> }>;
  paragraphRuns?: Array<{ start: number; end: number; style: Record<string, unknown> }>;
  contentAlignment?: string | null;
  fontScale?: number | null;
  x?: number | null;
  y?: number | null;
  width?: number | null;
  height?: number | null;
};

export type GoogleSlideRect = { x: number; y: number; width: number; height: number };
export type GoogleSlideResizeHandle = "move" | "ne" | "se" | "sw" | "nw";
type GoogleSlidePreviewItem = Extract<GoogleFilePreview, { kind: "presentation" }>["slides"][number];
type GoogleSlideSceneElement = NonNullable<GoogleSlidePreviewItem["elements"]>[number];
type LocalGoogleSlide = GoogleSlidePreviewItem & {
  optimistic?: boolean;
  optimisticThumbnail?: string | null;
};

function googleSlideObjectId(prefix: string) {
  return `milim_${prefix}_${crypto.randomUUID().replace(/-/g, "")}`;
}

export function googleSlideMarqueeRect(startX: number, startY: number, endX: number, endY: number): GoogleSlideRect {
  return {
    x: Math.min(startX, endX),
    y: Math.min(startY, endY),
    width: Math.abs(endX - startX),
    height: Math.abs(endY - startY),
  };
}

export function googleSlideRectsIntersect(a: GoogleSlideRect, b: GoogleSlideRect): boolean {
  return a.x <= b.x + b.width
    && a.x + a.width >= b.x
    && a.y <= b.y + b.height
    && a.y + a.height >= b.y;
}

export function googleSlideGroupBounds(rects: GoogleSlideRect[]): GoogleSlideRect | null {
  if (!rects.length) return null;
  const left = Math.min(...rects.map((rect) => rect.x));
  const top = Math.min(...rects.map((rect) => rect.y));
  const right = Math.max(...rects.map((rect) => rect.x + rect.width));
  const bottom = Math.max(...rects.map((rect) => rect.y + rect.height));
  return { x: left, y: top, width: right - left, height: bottom - top };
}

export function googleSlideTransformGroup(rect: GoogleSlideRect, from: GoogleSlideRect, to: GoogleSlideRect): GoogleSlideRect {
  const scaleX = to.width / from.width;
  const scaleY = to.height / from.height;
  return {
    x: to.x + (rect.x - from.x) * scaleX,
    y: to.y + (rect.y - from.y) * scaleY,
    width: rect.width * scaleX,
    height: rect.height * scaleY,
  };
}

export function googleSlideGestureRect(
  rect: GoogleSlideRect,
  deltaX: number,
  deltaY: number,
  handle: GoogleSlideResizeHandle,
): GoogleSlideRect {
  const minimum = 0.02;
  if (handle === "move") {
    return {
      ...rect,
      x: Math.min(1 - rect.width, Math.max(0, rect.x + deltaX)),
      y: Math.min(1 - rect.height, Math.max(0, rect.y + deltaY)),
    };
  }
  const left = handle.includes("w")
    ? Math.min(rect.x + rect.width - minimum, Math.max(0, rect.x + deltaX))
    : rect.x;
  const right = handle.includes("e")
    ? Math.max(rect.x + minimum, Math.min(1, rect.x + rect.width + deltaX))
    : rect.x + rect.width;
  const top = handle.includes("n")
    ? Math.min(rect.y + rect.height - minimum, Math.max(0, rect.y + deltaY))
    : rect.y;
  const bottom = handle.includes("s")
    ? Math.max(rect.y + minimum, Math.min(1, rect.y + rect.height + deltaY))
    : rect.y + rect.height;
  return { x: left, y: top, width: right - left, height: bottom - top };
}

export function googleSlideSnapRect(
  rect: GoogleSlideRect,
  otherRects: GoogleSlideRect[],
  handle: GoogleSlideResizeHandle,
  thresholdX: number,
  thresholdY: number,
): { rect: GoogleSlideRect; guideX?: number; guideY?: number } {
  const targetsX = [0, 0.5, 1, ...otherRects.flatMap((item) => [item.x, item.x + item.width / 2, item.x + item.width])];
  const targetsY = [0, 0.5, 1, ...otherRects.flatMap((item) => [item.y, item.y + item.height / 2, item.y + item.height])];
  const nearest = (anchors: number[], targets: number[], threshold: number) => {
    let match: { delta: number; guide: number } | null = null;
    for (const anchor of anchors) {
      for (const target of targets) {
        const delta = target - anchor;
        if (Math.abs(delta) <= threshold && (!match || Math.abs(delta) < Math.abs(match.delta)))
          match = { delta, guide: target };
      }
    }
    return match;
  };
  const xAnchors = handle === "move"
    ? [rect.x, rect.x + rect.width / 2, rect.x + rect.width]
    : handle.includes("w") ? [rect.x] : [rect.x + rect.width];
  const yAnchors = handle === "move"
    ? [rect.y, rect.y + rect.height / 2, rect.y + rect.height]
    : handle.includes("n") ? [rect.y] : [rect.y + rect.height];
  const snapX = handle === "move" || handle.includes("e") || handle.includes("w")
    ? nearest(xAnchors, targetsX, thresholdX)
    : null;
  const snapY = handle === "move" || handle.includes("n") || handle.includes("s")
    ? nearest(yAnchors, targetsY, thresholdY)
    : null;
  const next = { ...rect };
  if (snapX) {
    if (handle === "move") next.x += snapX.delta;
    else if (handle.includes("w")) {
      next.x += snapX.delta;
      next.width -= snapX.delta;
    } else next.width += snapX.delta;
  }
  if (snapY) {
    if (handle === "move") next.y += snapY.delta;
    else if (handle.includes("n")) {
      next.y += snapY.delta;
      next.height -= snapY.delta;
    } else next.height += snapY.delta;
  }
  return {
    rect: next,
    ...(snapX ? { guideX: snapX.guide } : {}),
    ...(snapY ? { guideY: snapY.guide } : {}),
  };
}

function googleSlideElementRect(element: GoogleSlideSceneElement): GoogleSlideRect | null {
  return element.x == null || element.y == null || element.width == null || element.height == null
    ? null
    : { x: element.x, y: element.y, width: element.width, height: element.height };
}

function googleSlideRectStyle(rect: GoogleSlideRect): CSSProperties {
  return {
    left: `${rect.x * 100}%`,
    top: `${rect.y * 100}%`,
    width: `${rect.width * 100}%`,
    height: `${rect.height * 100}%`,
  };
}

function googleSlideCropStyle(rect: GoogleSlideRect): CSSProperties {
  return {
    position: "absolute",
    left: `${-rect.x / rect.width * 100}%`,
    top: `${-rect.y / rect.height * 100}%`,
    width: `${100 / rect.width}%`,
    height: `${100 / rect.height}%`,
  };
}

function GoogleSlideInlineEditor({
  field,
  value,
  active,
  optimistic,
  style,
  onFocus,
  onInput,
  onSelection,
  onBlur,
  onEscape,
}: {
  field: GoogleSlideEditableField;
  value: string;
  active: boolean;
  optimistic: boolean;
  style: CSSProperties;
  onFocus: (element: HTMLDivElement) => void;
  onInput: (value: string) => void;
  onSelection: (element: HTMLDivElement) => void;
  onBlur: (event: FocusEvent<HTMLDivElement>) => void;
  onEscape: (element: HTMLDivElement) => void;
}) {
  const editorRef = useRef<HTMLDivElement | null>(null);
  const renderedStyleKeyRef = useRef("");
  const styleKey = JSON.stringify([
    field.styleRuns,
    field.paragraphRuns,
    field.contentAlignment,
    field.fontScale,
  ]);

  useEffect(() => {
    const editor = editorRef.current;
    if (!editor || active || (editor.textContent === value && renderedStyleKeyRef.current === styleKey)) return;
    editor.replaceChildren(...googleSlideTextSegments({ ...field, text: value }).map((segment) => {
      const span = document.createElement("span");
      span.textContent = segment.text;
      span.style.color = segment.format.color;
      span.style.fontSize = `${segment.format.fontSize * (field.fontScale ?? 1) / 7.2}cqw`;
      span.style.fontFamily = segment.format.fontFamily ?? "";
      span.style.fontWeight = segment.format.bold ? "700" : "";
      span.style.fontStyle = segment.format.italic ? "italic" : "";
      span.style.textDecoration = segment.format.underline ? "underline" : "";
      return span;
    }));
    const content = document.createElement("div");
    content.className = "google-slide-inline-editor-content";
    content.append(...editor.childNodes);
    editor.replaceChildren(content);
    renderedStyleKeyRef.current = styleKey;
  }, [active, field, styleKey, value]);

  return (
    <div
      ref={editorRef}
      className="google-slide-inline-editor"
      contentEditable
      suppressContentEditableWarning
      role="textbox"
      aria-label={`Edit ${field.label.toLocaleLowerCase()}`}
      aria-multiline="true"
      data-active={active ? "true" : undefined}
      data-optimistic={optimistic ? "true" : undefined}
      data-content-alignment={field.contentAlignment ?? undefined}
      style={style}
      onFocus={(event) => onFocus(event.currentTarget)}
      onInput={(event) => onInput(event.currentTarget.textContent?.replace(/\r\n?/g, "\n") ?? "")}
      onSelect={(event) => onSelection(event.currentTarget)}
      onKeyUp={(event) => onSelection(event.currentTarget)}
      onMouseUp={(event) => onSelection(event.currentTarget)}
      onPaste={(event) => {
        event.preventDefault();
        window.document.execCommand("insertText", false, event.clipboardData.getData("text/plain"));
      }}
      onBlur={onBlur}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          event.preventDefault();
          window.document.execCommand("insertText", false, "\n");
        } else if (event.key === "Escape") {
          event.preventDefault();
          onEscape(event.currentTarget);
          event.currentTarget.blur();
        }
      }}
    />
  );
}

export function SlidesPreview({
  fileId,
  slides,
  pageAspectRatio,
  pageWidth,
  pageHeight,
  active,
  canEdit,
}: {
  fileId: string;
  slides: Extract<GoogleFilePreview, { kind: "presentation" }>["slides"];
  pageAspectRatio: number;
  pageWidth: number;
  pageHeight: number;
  active: boolean;
  canEdit: boolean;
  onSaved: () => void;
}) {
  const [selected, setSelected] = useState(0);
  const [visibleSlides, setVisibleSlides] = useState<LocalGoogleSlide[]>(slides);
  const slide = visibleSlides[selected] ?? visibleSlides[0];
  const [thumbnail, setThumbnail] = useState<string | null>(null);
  const [thumbnailError, setThumbnailError] = useState<string | null>(null);
  const thumbnailObjectUrlRef = useRef<string | null>(null);
  const thumbnailRequestRef = useRef<string | null>(null);
  const presentationReturnFocusRef = useRef<HTMLElement | null>(null);
  const presentationRef = useRef<HTMLDivElement | null>(null);
  const [query, setQuery] = useState("");
  const [zoom, setZoom] = useState<"fit" | number>("fit");
  const [railOpen, setRailOpen] = useState(true);
  const [notesOpen, setNotesOpen] = useState(false);
  const [presenting, setPresenting] = useState(false);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [savedDrafts, setSavedDrafts] = useState<Record<string, string>>({});
  const [formatOverrides, setFormatOverrides] = useState<Record<string, GoogleTextFormat>>({});
  const [activeTextFieldId, setActiveTextFieldId] = useState<string | null>(null);
  const [textSelection, setTextSelection] = useState({ start: 0, end: 0 });
  const [slideFormat, setSlideFormat] = useState<GoogleTextFormat | null>(null);
  const [selectedElementIds, setSelectedElementIds] = useState<string[]>([]);
  const [geometryOverrides, setGeometryOverrides] = useState<Record<string, GoogleSlideRect>>({});
  const [geometrySourceRects, setGeometrySourceRects] = useState<Record<string, GoogleSlideRect>>({});
  const [optimisticTextFieldIds, setOptimisticTextFieldIds] = useState<Set<string>>(() => new Set());
  const [snapGuides, setSnapGuides] = useState<{ x?: number; y?: number }>({});
  const [marqueeRect, setMarqueeRect] = useState<GoogleSlideRect | null>(null);
  const [thumbnailRequest, setThumbnailRequest] = useState({ generation: 0, reconcileRevision: 0 });
  const optimisticRevisionRef = useRef(0);
  const queuedRevisionRef = useRef(0);
  const reconciledRevisionRef = useRef(0);
  const pendingReconcileRevisionRef = useRef(0);
  const serverSlidesRef = useRef(slides);
  const saveQueue = useGoogleSaveQueue(() => {
    pendingReconcileRevisionRef.current = queuedRevisionRef.current;
  });
  const canvasRef = useRef<HTMLDivElement | null>(null);
  const draftSlideRef = useRef<string | null>(null);
  const savedDraftsRef = useRef<Record<string, string>>({});
  const activeSlideEditorRef = useRef<HTMLDivElement | null>(null);
  const slideSelectionRangeRef = useRef<Range | null>(null);
  const slidesSourceRef = useRef(slides);
  const draftCacheRef = useRef<Record<string, {
    drafts: Record<string, string>;
    savedDrafts: Record<string, string>;
    formats: Record<string, GoogleTextFormat>;
  }>>({});
  const editableFields = useMemo(() => {
    if (!slide) return [];
    const fields: GoogleSlideEditableField[] = (slide.textElements ?? []).map((element, index) => ({
      id: element.objectId,
      label: `Text ${index + 1}`,
      text: element.text,
      kind: "shape" as const,
      styleRuns: element.styleRuns,
      paragraphRuns: element.paragraphRuns,
      contentAlignment: element.contentAlignment,
      fontScale: element.fontScale,
      x: element.x,
      y: element.y,
      width: element.width,
      height: element.height,
    }));
    if (slide.notesObjectId) {
      fields.push({
        id: slide.notesObjectId,
        label: "Speaker notes",
        text: slide.notes ?? "",
        kind: "notes" as const,
        x: null,
        y: null,
        width: null,
        height: null,
      });
    }
    return fields;
  }, [slide]);
  const matchingSlides = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    if (!normalized) return [];
    return visibleSlides.flatMap((item, index) => `${item.text}\n${item.notes ?? ""}`.toLocaleLowerCase().includes(normalized) ? [index] : []);
  }, [query, visibleSlides]);

  useEffect(() => {
    if (selected < visibleSlides.length) return;
    setSelected(Math.max(0, visibleSlides.length - 1));
  }, [selected, visibleSlides.length]);

  useEffect(() => {
    if (slidesSourceRef.current === slides) return;
    slidesSourceRef.current = slides;
    serverSlidesRef.current = slides;
    if (saveQueue.pending > 0) return;
    const pendingRevision = pendingReconcileRevisionRef.current;
    if (pendingRevision > reconciledRevisionRef.current) {
      setVisibleSlides((current) => current.map((item) => (
        item.optimistic
          ? slides.find((candidate) => candidate.objectId === item.objectId) ?? item
          : item
      )));
      setThumbnailRequest((current) => ({
        generation: current.generation + 1,
        reconcileRevision: pendingRevision,
      }));
      return;
    }
    if (optimisticRevisionRef.current === reconciledRevisionRef.current) {
      setVisibleSlides(slides);
      setThumbnailRequest((current) => ({
        generation: current.generation + 1,
        reconcileRevision: reconciledRevisionRef.current,
      }));
      draftCacheRef.current = {};
    }
  }, [saveQueue.pending, slides]);

  useEffect(() => {
    const next = Object.fromEntries(editableFields.map((field) => [field.id, field.text]));
    const slideKey = slide?.objectId ?? String(selected);
    if (draftSlideRef.current !== slideKey) {
      const cached = draftCacheRef.current[slideKey];
      draftSlideRef.current = slideKey;
      setDrafts(cached?.drafts ?? next);
      setSavedDrafts(cached?.savedDrafts ?? next);
      setFormatOverrides(cached?.formats ?? {});
      setActiveTextFieldId(null);
      setSlideFormat(null);
      savedDraftsRef.current = cached?.savedDrafts ?? next;
      return;
    }
    if (saveQueue.pending > 0) return;
    setDrafts((current) => Object.keys(current).some((id) => current[id] !== savedDraftsRef.current[id])
      ? current
      : next);
    setSavedDrafts(next);
    setFormatOverrides({});
    setSlideFormat(null);
    savedDraftsRef.current = next;
  }, [editableFields, selected, slide?.objectId]);

  useEffect(() => {
    thumbnailRequestRef.current = null;
    setThumbnail(null);
  }, [slide?.objectId]);

  useEffect(() => {
    if (!slide?.objectId) return;
    if (slide.optimistic) {
      setThumbnail(slide.optimisticThumbnail ?? null);
      setThumbnailError(null);
      return;
    }
    const requestKey = googleSlideThumbnailRequestKey(fileId, slide.objectId, thumbnailRequest.generation);
    if (!googleWorkspacePreviewNeedsLoad(active, thumbnailRequestRef.current, requestKey))
      return;
    thumbnailRequestRef.current = requestKey;
    let cancelled = false;
    let objectUrl: string | null = null;
    setThumbnailError(null);
    getGoogleFileContent(fileId, slide.objectId)
      .then((blob) => {
        if (cancelled) return;
        objectUrl = URL.createObjectURL(blob);
        if (thumbnailRequest.reconcileRevision !== optimisticRevisionRef.current) {
          URL.revokeObjectURL(objectUrl);
          objectUrl = null;
          return;
        }
        if (thumbnailObjectUrlRef.current)
          URL.revokeObjectURL(thumbnailObjectUrlRef.current);
        thumbnailObjectUrlRef.current = objectUrl;
        setThumbnail(objectUrl);
        reconciledRevisionRef.current = thumbnailRequest.reconcileRevision;
        if (pendingReconcileRevisionRef.current <= thumbnailRequest.reconcileRevision)
          pendingReconcileRevisionRef.current = 0;
        setVisibleSlides(serverSlidesRef.current);
        setGeometryOverrides({});
        setGeometrySourceRects({});
        setOptimisticTextFieldIds(new Set());
        draftCacheRef.current = {};
      })
      .catch((cause) => {
        if (!cancelled) setThumbnailError(cause instanceof Error ? cause.message : String(cause));
      });
    return () => {
      cancelled = true;
      if (!objectUrl && thumbnailRequestRef.current === requestKey)
        thumbnailRequestRef.current = null;
    };
  }, [active, fileId, slide?.objectId, slide?.optimistic, slide?.optimisticThumbnail, thumbnailRequest]);

  useEffect(() => () => {
    if (thumbnailObjectUrlRef.current)
      URL.revokeObjectURL(thumbnailObjectUrlRef.current);
  }, []);

  const hasSlideChanges = editableFields.some((field) => (drafts[field.id] ?? "") !== (savedDrafts[field.id] ?? ""));

  function enqueueSlideSave(task: () => Promise<void>) {
    queuedRevisionRef.current = optimisticRevisionRef.current;
    saveQueue.enqueue(task);
  }

  function queueSlideText() {
    const snapshot = { ...drafts };
    const savingSlideKey = slide?.objectId ?? String(selected);
    const operations: GoogleSlidesEditOperation[] = [];
    for (const field of editableFields) {
      const draft = snapshot[field.id] ?? "";
      const saved = savedDrafts[field.id] ?? "";
      const replacement = googleDocTextReplacement(0, saved, draft);
      if (!replacement) continue;
      if (replacement.end > replacement.start) {
        operations.push({
          action: "delete_text",
          object_id: field.id,
          start: replacement.start,
          end: replacement.end,
        });
      }
      if (replacement.text) {
        operations.push({
          action: "insert_text",
          object_id: field.id,
          offset: replacement.start,
          text: replacement.text,
        });
      }
    }
    const nextSaved = operations.length ? snapshot : { ...savedDrafts };
    draftCacheRef.current[savingSlideKey] = {
      drafts: snapshot,
      savedDrafts: nextSaved,
      formats: { ...formatOverrides },
    };
    if (!operations.length) {
      if (
        optimisticTextFieldIds.size === 0
        && Object.keys(geometryOverrides).length === 0
        && pendingReconcileRevisionRef.current === 0
        && saveQueue.pending === 0
      )
        reconciledRevisionRef.current = optimisticRevisionRef.current;
      return;
    }
    setSavedDrafts(snapshot);
    savedDraftsRef.current = snapshot;
    enqueueSlideSave(() => editGoogleSlides(fileId, operations));
  }

  useEffect(() => {
    if (!hasSlideChanges) return;
    const timeout = window.setTimeout(queueSlideText, 800);
    return () => window.clearTimeout(timeout);
  }, [drafts, hasSlideChanges, slide?.objectId]);

  const queueSlideTextRef = useRef(queueSlideText);
  queueSlideTextRef.current = queueSlideText;
  const navigateToSlide = useCallback((next: number) => {
    queueSlideTextRef.current();
    setSelectedElementIds([]);
    setSelected(Math.min(visibleSlides.length - 1, Math.max(0, next)));
  }, [visibleSlides.length]);

  function selectRelative(delta: -1 | 1) {
    navigateToSlide(selected + delta);
  }

  const applyNavigationAction = useCallback((action: GoogleSlidesNavigationAction) => {
    if (action === "previous") navigateToSlide(selected - 1);
    else if (action === "next") navigateToSlide(selected + 1);
    else if (action === "first") navigateToSlide(0);
    else if (action === "last") navigateToSlide(visibleSlides.length - 1);
  }, [navigateToSlide, selected, visibleSlides.length]);

  useEffect(() => {
    if (!presenting) return;
    const appRoot = document.querySelector<HTMLElement>(".app");
    const appWasInert = appRoot?.inert ?? false;
    const appAriaHidden = appRoot?.getAttribute("aria-hidden") ?? null;
    if (appRoot) {
      appRoot.inert = true;
      appRoot.setAttribute("aria-hidden", "true");
    }
    presentationRef.current?.focus();

    let cancelled = false;
    let fullscreenWindow: {
      isFullscreen: () => Promise<boolean>;
      setFullscreen: (fullscreen: boolean) => Promise<void>;
    } | null = null;
    let changedFullscreen = false;
    void (async () => {
      if (!("__TAURI_INTERNALS__" in window)) return;
      try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        fullscreenWindow = getCurrentWindow();
        const wasFullscreen = await fullscreenWindow.isFullscreen();
        if (cancelled || wasFullscreen) return;
        await fullscreenWindow.setFullscreen(true);
        if (cancelled) {
          await fullscreenWindow.setFullscreen(false);
          return;
        }
        changedFullscreen = true;
      } catch {
        // The fixed overlay remains usable when native fullscreen is unavailable.
      }
    })();

    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      const action = googleSlidesNavigationAction(event.key);
      if (!action) return;
      if (action === "next" && event.key === " " && event.target instanceof HTMLButtonElement)
        return;
      event.preventDefault();
      if (action === "exit") setPresenting(false);
      else applyNavigationAction(action);
    };
    document.addEventListener("keydown", onKeyDown);
    return () => {
      cancelled = true;
      document.removeEventListener("keydown", onKeyDown);
      if (changedFullscreen && fullscreenWindow)
        void fullscreenWindow.setFullscreen(false).catch(() => {});
      if (appRoot) {
        appRoot.inert = appWasInert;
        if (appAriaHidden === null) appRoot.removeAttribute("aria-hidden");
        else appRoot.setAttribute("aria-hidden", appAriaHidden);
      }
      presentationReturnFocusRef.current?.focus();
    };
  }, [applyNavigationAction, presenting]);

  function selectNextMatch(reverse = false) {
    if (!matchingSlides.length) return;
    const current = matchingSlides.indexOf(selected);
    const next = reverse
      ? (current <= 0 ? matchingSlides.length : current) - 1
      : (current + 1) % matchingSlides.length;
    navigateToSlide(matchingSlides[next]);
  }

  const shapeFields = editableFields.filter((field) => field.kind === "shape");
  const notesField = editableFields.find((field) => field.kind === "notes");
  const activeTextField = shapeFields.find((field) => field.id === activeTextFieldId);
  const activeTextRange = activeTextField && textSelection.end > textSelection.start
    ? textSelection
    : null;
  const activeTextFormat = slideFormat
    ?? (activeTextField ? formatOverrides[activeTextField.id] ?? googleSlideTextFormat(activeTextField, textSelection.start) : null);
  const sceneElements: GoogleSlideSceneElement[] = slide?.elements?.length
    ? slide.elements
    : shapeFields.flatMap((field, order) => (
      field.x == null || field.y == null || field.width == null || field.height == null
        ? []
        : [{
          objectId: field.id,
          kind: "shape" as const,
          order,
          x: field.x,
          y: field.y,
          width: field.width,
          height: field.height,
          baseWidth: field.width * pageWidth,
          baseHeight: field.height * pageHeight,
        }]
    ));
  const selectedElements = sceneElements.filter((element) => selectedElementIds.includes(element.objectId));
  const selectedElementRects = selectedElements.flatMap((element) => {
    const rect = geometryOverrides[element.objectId] ?? googleSlideElementRect(element);
    return rect ? [{ element, rect }] : [];
  });
  const selectedGroupRect = selectedElementRects.length > 1
    ? googleSlideGroupBounds(selectedElementRects.map(({ rect }) => rect))
    : null;
  const selectedElement = selectedElements[0];
  const selectedElementRect = selectedElement
    ? geometryOverrides[selectedElement.objectId] ?? googleSlideElementRect(selectedElement)
    : null;

  function updateSlideSelection(field: GoogleSlideEditableField, element: HTMLDivElement) {
    const selection = window.getSelection();
    if (!selection || selection.rangeCount === 0) return;
    const range = selection.getRangeAt(0);
    if (!element.contains(range.startContainer) || !element.contains(range.endContainer)) return;
    const prefix = range.cloneRange();
    prefix.selectNodeContents(element);
    prefix.setEnd(range.startContainer, range.startOffset);
    const nextRange = googleDocSelectionRange(0, prefix.toString(), range.toString());
    setTextSelection(nextRange);
    slideSelectionRangeRef.current = range.cloneRange();
    setSlideFormat(googleSlideTextFormat(field, nextRange.start));
  }

  function restoreSlideSelection() {
    const range = slideSelectionRangeRef.current;
    if (!range) return;
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
  }

  function markSlideOptimistic(textFieldId?: string) {
    optimisticRevisionRef.current += 1;
    if (textFieldId) {
      setOptimisticTextFieldIds((current) => {
        if (current.has(textFieldId)) return current;
        const next = new Set(current);
        next.add(textFieldId);
        return next;
      });
    }
  }

  function clearOptimisticTextField(textFieldId: string) {
    setOptimisticTextFieldIds((current) => {
      if (!current.has(textFieldId)) return current;
      const next = new Set(current);
      next.delete(textFieldId);
      if (
        next.size === 0
        && Object.keys(geometryOverrides).length === 0
        && pendingReconcileRevisionRef.current === 0
        && saveQueue.pending === 0
      )
        reconciledRevisionRef.current = optimisticRevisionRef.current;
      return next;
    });
  }

  function applySlideTextFormat(change: GoogleTextFormatChange) {
    if (!activeTextField || !activeTextRange || !activeTextFormat) return;
    queueSlideText();
    markSlideOptimistic(activeTextField.id);
    restoreSlideSelection();
    if (change.field === "fontSize") {
      window.document.execCommand("fontSize", false, "7");
      activeSlideEditorRef.current?.querySelectorAll<HTMLElement>('font[size="7"]').forEach((element) => {
        element.removeAttribute("size");
        element.style.fontSize = `${change.value * (activeTextField.fontScale ?? 1) / 7.2}cqw`;
      });
    } else if (change.field === "color") {
      window.document.execCommand("foreColor", false, change.value);
    } else if (change.field === "alignment") {
      window.document.execCommand(
        change.value === "CENTER"
          ? "justifyCenter"
          : change.value === "END"
            ? "justifyRight"
            : change.value === "JUSTIFIED"
              ? "justifyFull"
              : "justifyLeft",
      );
    } else {
      window.document.execCommand(change.field);
    }
    const operation: GoogleSlidesEditOperation = change.field === "alignment"
      ? {
        action: "set_paragraph_style",
        object_id: activeTextField.id,
        ...activeTextRange,
        alignment: change.value,
      }
      : {
        action: "set_text_style",
        object_id: activeTextField.id,
        ...activeTextRange,
        ...(change.field === "fontSize"
          ? { font_size: change.value }
          : change.field === "color"
            ? { foreground_color: change.value }
            : { [change.field]: change.value }),
      };
    const nextFormat = { ...activeTextFormat, [change.field]: change.value } as GoogleTextFormat;
    setSlideFormat(nextFormat);
    setFormatOverrides((current) => ({ ...current, [activeTextField.id]: nextFormat }));
    enqueueSlideSave(() => editGoogleSlides(fileId, [operation]));
  }

  function updateCurrentSlide(update: (item: LocalGoogleSlide) => LocalGoogleSlide) {
    setVisibleSlides((current) => current.map((item, index) => index === selected ? update(item) : item));
  }

  function createSlide() {
    queueSlideText();
    markSlideOptimistic();
    const objectId = googleSlideObjectId("slide");
    const insertionIndex = selected + 1;
    const next: LocalGoogleSlide = {
      objectId,
      text: "",
      notes: "",
      notesObjectId: null,
      textElements: [],
      elements: [],
      optimistic: true,
      optimisticThumbnail: null,
    };
    setVisibleSlides((current) => [...current.slice(0, insertionIndex), next, ...current.slice(insertionIndex)]);
    setSelected(insertionIndex);
    enqueueSlideSave(() => editGoogleSlides(fileId, [{
      action: "create_slide",
      object_id: objectId,
      layout: "BLANK",
      insertion_index: insertionIndex,
    }]));
  }

  function duplicateSlide(index = selected) {
    const source = visibleSlides[index];
    if (!source?.objectId) return;
    queueSlideText();
    markSlideOptimistic();
    const objectId = googleSlideObjectId("slide");
    const insertionIndex = index + 1;
    const next: LocalGoogleSlide = {
      ...source,
      objectId,
      textElements: [],
      elements: [],
      optimistic: true,
      optimisticThumbnail: index === selected ? thumbnail : null,
    };
    setVisibleSlides((current) => [...current.slice(0, insertionIndex), next, ...current.slice(insertionIndex)]);
    setSelected(index === selected ? insertionIndex : insertionIndex <= selected ? selected + 1 : selected);
    enqueueSlideSave(() => editGoogleSlides(fileId, [{
      action: "duplicate_slide",
      object_id: source.objectId!,
      new_object_id: objectId,
    }]));
  }

  function deleteSlide(index = selected) {
    const target = visibleSlides[index];
    if (!target?.objectId || visibleSlides.length <= 1 || !window.confirm(`Delete slide ${index + 1}?`)) return;
    queueSlideText();
    markSlideOptimistic();
    const objectId = target.objectId;
    setVisibleSlides((current) => current.filter((_, itemIndex) => itemIndex !== index));
    setSelected(index < selected ? selected - 1 : index === selected ? Math.min(selected, visibleSlides.length - 2) : selected);
    enqueueSlideSave(() => editGoogleSlides(fileId, [{ action: "delete_slide", object_id: objectId }]));
  }

  function moveSlide(delta: -1 | 1) {
    const destination = selected + delta;
    if (!slide?.objectId || destination < 0 || destination >= visibleSlides.length) return;
    queueSlideText();
    markSlideOptimistic();
    setVisibleSlides((current) => {
      const next = [...current];
      const [moved] = next.splice(selected, 1);
      next.splice(destination, 0, moved);
      return next;
    });
    setSelected(destination);
    enqueueSlideSave(() => editGoogleSlides(fileId, [{
      action: "reorder_slides",
      slide_object_ids: [slide.objectId!],
      insertion_index: destination > selected ? destination + 1 : destination,
    }]));
  }

  function commitElementRects(entries: Array<{ element: GoogleSlideSceneElement; rect: GoogleSlideRect }>) {
    const rects = Object.fromEntries(entries.map(({ element, rect }) => [element.objectId, rect]));
    updateCurrentSlide((item) => ({
      ...item,
      elements: item.elements?.map((candidate) => rects[candidate.objectId] ? { ...candidate, ...rects[candidate.objectId] } : candidate),
      textElements: item.textElements?.map((candidate) => rects[candidate.objectId] ? { ...candidate, ...rects[candidate.objectId] } : candidate),
    }));
    enqueueSlideSave(() => editGoogleSlides(fileId, entries.map(({ element, rect }) => ({
      action: "update_element_transform",
      object_id: element.objectId,
      x: rect.x * pageWidth,
      y: rect.y * pageHeight,
      width: rect.width * pageWidth,
      height: rect.height * pageHeight,
      base_width: element.baseWidth!,
      base_height: element.baseHeight!,
    }))));
  }

  function commitElementRect(element: GoogleSlideSceneElement, rect: GoogleSlideRect) {
    if (!element.baseWidth || !element.baseHeight) return;
    commitElementRects([{ element, rect }]);
  }

  function startElementGesture(
    event: ReactPointerEvent<HTMLButtonElement>,
    element: GoogleSlideSceneElement,
    handle: GoogleSlideResizeHandle,
  ) {
    if (event.button !== 0 || !canvasRef.current) return;
    const startRect = geometryOverrides[element.objectId] ?? googleSlideElementRect(element);
    if (!startRect || !element.baseWidth || !element.baseHeight) return;
    event.preventDefault();
    event.stopPropagation();
    queueSlideText();
    activeSlideEditorRef.current?.blur();
    setSelectedElementIds([element.objectId]);
    setGeometrySourceRects((current) => current[element.objectId]
      ? current
      : { ...current, [element.objectId]: googleSlideElementRect(element)! });
    const bounds = canvasRef.current.getBoundingClientRect();
    const origin = { x: event.clientX, y: event.clientY };
    const otherRects = sceneElements.flatMap((item) => {
      if (item.objectId === element.objectId) return [];
      const rect = geometryOverrides[item.objectId] ?? googleSlideElementRect(item);
      return rect ? [rect] : [];
    });
    let latest = startRect;
    const onMove = (moveEvent: globalThis.PointerEvent) => {
      const raw = googleSlideGestureRect(
        startRect,
        (moveEvent.clientX - origin.x) / bounds.width,
        (moveEvent.clientY - origin.y) / bounds.height,
        handle,
      );
      const snapped = googleSlideSnapRect(raw, otherRects, handle, 6 / bounds.width, 6 / bounds.height);
      latest = snapped.rect;
      setGeometryOverrides((current) => ({ ...current, [element.objectId]: latest }));
      setSnapGuides({ x: snapped.guideX, y: snapped.guideY });
    };
    const onEnd = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onEnd);
      window.removeEventListener("pointercancel", onEnd);
      setSnapGuides({});
      if (latest !== startRect) {
        markSlideOptimistic();
        commitElementRect(element, latest);
      }
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onEnd);
    window.addEventListener("pointercancel", onEnd);
  }

  function startGroupGesture(event: ReactPointerEvent<HTMLButtonElement>, handle: GoogleSlideResizeHandle) {
    if (event.button !== 0 || !canvasRef.current || !selectedGroupRect || selectedElementRects.some(({ element }) => !element.baseWidth || !element.baseHeight))
      return;
    event.preventDefault();
    event.stopPropagation();
    queueSlideText();
    activeSlideEditorRef.current?.blur();
    const bounds = canvasRef.current.getBoundingClientRect();
    const origin = { x: event.clientX, y: event.clientY };
    const startRects = selectedElementRects;
    const selectedIds = new Set(selectedElementIds);
    const otherRects = sceneElements.flatMap((element) => {
      if (selectedIds.has(element.objectId)) return [];
      const rect = geometryOverrides[element.objectId] ?? googleSlideElementRect(element);
      return rect ? [rect] : [];
    });
    setGeometrySourceRects((current) => ({
      ...Object.fromEntries(startRects.map(({ element }) => [
        element.objectId,
        current[element.objectId] ?? googleSlideElementRect(element),
      ]).filter((entry): entry is [string, GoogleSlideRect] => Boolean(entry[1]))),
      ...current,
    }));
    let latest = startRects;
    let changed = false;
    const onMove = (moveEvent: globalThis.PointerEvent) => {
      const raw = googleSlideGestureRect(
        selectedGroupRect,
        (moveEvent.clientX - origin.x) / bounds.width,
        (moveEvent.clientY - origin.y) / bounds.height,
        handle,
      );
      const snapped = googleSlideSnapRect(raw, otherRects, handle, 6 / bounds.width, 6 / bounds.height);
      latest = startRects.map(({ element, rect }) => ({
        element,
        rect: googleSlideTransformGroup(rect, selectedGroupRect, snapped.rect),
      }));
      changed = true;
      setGeometryOverrides((current) => ({
        ...current,
        ...Object.fromEntries(latest.map(({ element, rect }) => [element.objectId, rect])),
      }));
      setSnapGuides({ x: snapped.guideX, y: snapped.guideY });
    };
    const onEnd = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onEnd);
      window.removeEventListener("pointercancel", onEnd);
      setSnapGuides({});
      if (changed) {
        markSlideOptimistic();
        commitElementRects(latest);
      }
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onEnd);
    window.addEventListener("pointercancel", onEnd);
  }

  function startMarquee(event: ReactPointerEvent<HTMLDivElement>) {
    if (!canEdit || event.button !== 0 || !canvasRef.current) return;
    const target = event.target;
    if (target instanceof Element && target.closest(".google-slide-inline-editor, .google-slide-element-hit-target, .google-slide-selection-frame"))
      return;
    event.preventDefault();
    const bounds = canvasRef.current.getBoundingClientRect();
    const point = (clientX: number, clientY: number) => ({
      x: Math.min(1, Math.max(0, (clientX - bounds.left) / bounds.width)),
      y: Math.min(1, Math.max(0, (clientY - bounds.top) / bounds.height)),
    });
    const origin = point(event.clientX, event.clientY);
    let latest = googleSlideMarqueeRect(origin.x, origin.y, origin.x, origin.y);
    setSelectedElementIds([]);
    setMarqueeRect(latest);
    const onMove = (moveEvent: globalThis.PointerEvent) => {
      const current = point(moveEvent.clientX, moveEvent.clientY);
      latest = googleSlideMarqueeRect(origin.x, origin.y, current.x, current.y);
      setMarqueeRect(latest);
    };
    const onEnd = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onEnd);
      window.removeEventListener("pointercancel", onCancel);
      setMarqueeRect(null);
      if (latest.width * bounds.width < 3 && latest.height * bounds.height < 3) return;
      setSelectedElementIds(sceneElements.flatMap((element) => {
        const rect = geometryOverrides[element.objectId] ?? googleSlideElementRect(element);
        return rect && googleSlideRectsIntersect(latest, rect) ? [element.objectId] : [];
      }));
    };
    const onCancel = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onEnd);
      window.removeEventListener("pointercancel", onCancel);
      setMarqueeRect(null);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onEnd);
    window.addEventListener("pointercancel", onCancel);
  }

  function duplicateElement() {
    if (!selectedElement || !selectedElementRect || !selectedElement.baseWidth || !selectedElement.baseHeight) return;
    markSlideOptimistic();
    const objectId = googleSlideObjectId("element");
    const rect = googleSlideGestureRect(selectedElementRect, 0.02, 0.02, "move");
    const selectedTextElement = slide?.textElements?.find((element) => element.objectId === selectedElement.objectId);
    if (selectedTextElement) {
      setDrafts((current) => ({ ...current, [objectId]: current[selectedElement.objectId] ?? selectedTextElement.text }));
      setSavedDrafts((current) => ({ ...current, [objectId]: current[selectedElement.objectId] ?? selectedTextElement.text }));
      savedDraftsRef.current[objectId] = savedDraftsRef.current[selectedElement.objectId] ?? selectedTextElement.text;
    }
    updateCurrentSlide((item) => ({
      ...item,
      elements: [...sceneElements, { ...selectedElement, objectId, order: sceneElements.length }],
      textElements: selectedTextElement
        ? [...(item.textElements ?? []), { ...selectedTextElement, objectId }]
        : item.textElements,
    }));
    setGeometryOverrides((current) => ({ ...current, [objectId]: rect }));
    setSelectedElementIds([objectId]);
    enqueueSlideSave(() => editGoogleSlides(fileId, [
      { action: "duplicate_element", object_id: selectedElement.objectId, new_object_id: objectId },
      {
        action: "update_element_transform",
        object_id: objectId,
        x: rect.x * pageWidth,
        y: rect.y * pageHeight,
        width: rect.width * pageWidth,
        height: rect.height * pageHeight,
        base_width: selectedElement.baseWidth!,
        base_height: selectedElement.baseHeight!,
      },
    ]));
  }

  function deleteElement() {
    if (!selectedElement || !window.confirm(`Delete selected ${selectedElement.kind}?`)) return;
    markSlideOptimistic();
    updateCurrentSlide((item) => ({
      ...item,
      elements: item.elements?.filter((element) => element.objectId !== selectedElement.objectId),
      textElements: item.textElements?.filter((element) => element.objectId !== selectedElement.objectId),
    }));
    setSelectedElementIds([]);
    enqueueSlideSave(() => editGoogleSlides(fileId, [{
      action: "delete_element",
      object_id: selectedElement.objectId,
    }]));
  }

  if (!visibleSlides.length) return <div className="google-workspace-state"><strong>This presentation is empty.</strong></div>;

  return (
    <div
      className="google-slides-viewer"
      tabIndex={0}
      aria-label="Presentation viewer. Use arrow keys to change slides."
      onKeyDown={(event) => {
        if (["INPUT", "SELECT", "TEXTAREA", "BUTTON"].includes((event.target as HTMLElement).tagName)) return;
        const action = googleSlidesNavigationAction(event.key);
        if (!action || action === "exit") return;
        event.preventDefault();
        applyNavigationAction(action);
      }}
    >
      <div className="google-viewer-toolbar google-slides-unified-toolbar" role="toolbar" aria-label="Presentation controls">
        <button
          className={`preview-browser-action${railOpen ? " active" : ""}`}
          type="button"
          title={railOpen ? "Hide slide thumbnails" : "Show slide thumbnails"}
          aria-label={railOpen ? "Hide slide thumbnails" : "Show slide thumbnails"}
          aria-pressed={railOpen}
          onClick={() => setRailOpen((value) => !value)}
        >
          <Sidebar size={13} />
        </button>
        <button className="preview-browser-action" type="button" aria-label="Previous slide" disabled={selected === 0} onClick={() => selectRelative(-1)}>
          <ArrowLeft size={13} />
        </button>
        <strong className="google-slide-counter">{selected + 1} / {visibleSlides.length}</strong>
        <button className="preview-browser-action" type="button" aria-label="Next slide" disabled={selected === visibleSlides.length - 1} onClick={() => selectRelative(1)}>
          <ArrowRight size={13} />
        </button>
        {canEdit ? (
          <span className="google-slides-structure-tools">
            <button className="preview-browser-action" type="button" title="New slide" aria-label="New slide" onClick={createSlide}>
              <Plus size={13} />
            </button>
            <button className="preview-browser-action" type="button" title="Duplicate slide" aria-label="Duplicate slide" onClick={() => duplicateSlide()}>
              <Copy size={13} />
            </button>
            <button className="preview-browser-action" type="button" title="Move slide earlier" aria-label="Move slide earlier" disabled={selected === 0} onClick={() => moveSlide(-1)}>
              <ArrowLeft size={13} />
            </button>
            <button className="preview-browser-action" type="button" title="Move slide later" aria-label="Move slide later" disabled={selected === visibleSlides.length - 1} onClick={() => moveSlide(1)}>
              <ArrowRight size={13} />
            </button>
            <button className="preview-browser-action danger" type="button" title="Delete slide" aria-label="Delete slide" disabled={visibleSlides.length <= 1} onClick={() => deleteSlide()}>
              <Trash size={13} />
            </button>
          </span>
        ) : null}
        <label className="google-viewer-search">
          <Search size={13} aria-hidden="true" />
          <input
            aria-label="Search slides"
            placeholder="Find in slides"
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              const normalized = event.target.value.trim().toLocaleLowerCase();
              const firstMatch = visibleSlides.findIndex((item) => `${item.text}\n${item.notes ?? ""}`.toLocaleLowerCase().includes(normalized));
              if (normalized && firstMatch >= 0) navigateToSlide(firstMatch);
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
        {canEdit && activeTextRange && activeTextFormat ? (
          <GoogleTextFormatToolbar
            format={activeTextFormat}
            className="google-slides-contextual-tools"
            onChange={applySlideTextFormat}
          />
        ) : null}
        {canEdit && selectedElements.length === 1 && selectedElement ? (
          <span className="google-slides-element-tools">
            <button className="preview-browser-action" type="button" title="Duplicate selected element" aria-label="Duplicate selected element" onClick={duplicateElement}>
              <Copy size={13} />
            </button>
            <button className="preview-browser-action danger" type="button" title="Delete selected element" aria-label="Delete selected element" onClick={deleteElement}>
              <Trash size={13} />
            </button>
          </span>
        ) : canEdit && selectedElements.length > 1 ? <span className="google-viewer-match-count">{selectedElements.length} selected</span> : null}
        <span className="google-viewer-toolbar-spacer" />
        {visibleSlides.some((item) => item.notesObjectId || item.notes?.trim()) ? (
          <button
            className={`google-viewer-text-button${notesOpen ? " active" : ""}`}
            type="button"
            aria-pressed={notesOpen}
            onClick={() => {
              if (notesOpen) queueSlideText();
              setNotesOpen((value) => !value);
            }}
          >
            Notes
          </button>
        ) : null}
        <GoogleAutosaveStatus queue={saveQueue} dirty={hasSlideChanges} />
        <button
          className="google-viewer-present-button"
          type="button"
          aria-haspopup="dialog"
          onClick={(event) => {
            queueSlideText();
            presentationReturnFocusRef.current = event.currentTarget;
            setPresenting(true);
          }}
        >
          Present
        </button>
        <label className="google-viewer-zoom">
          <span>Zoom</span>
          <select
            aria-label="Slide zoom"
            value={zoom}
            onChange={(event) => setZoom(event.target.value === "fit" ? "fit" : Number(event.target.value))}
          >
            <option value="fit">Fit</option>
            {[75, 100, 125, 150, 200].map((value) => <option value={value} key={value}>{value}%</option>)}
          </select>
        </label>
      </div>
      <div className={`google-slides-preview${railOpen ? "" : " rail-collapsed"}`}>
      {railOpen ? <nav className="google-slide-rail" aria-label="Slides">
        {visibleSlides.map((item, index) => (
          <div
            className={`google-slide-rail-item${index === selected ? " active" : ""}${matchingSlides.includes(index) ? " match" : ""}`}
            key={item.objectId ?? index}
          >
            <button
              className="google-slide-rail-main"
              type="button"
              aria-current={index === selected ? "page" : undefined}
              aria-label={`Slide ${index + 1}`}
              onClick={() => navigateToSlide(index)}
            >
              <span className="google-slide-rail-number">{index + 1}</span>
              <SlideRailThumbnail
                fileId={fileId}
                objectId={item.objectId}
                active={active}
                optimistic={item.optimistic === true}
                revision={thumbnailRequest.generation}
                selectedSource={index === selected ? thumbnail : null}
                slide={item}
                drafts={index === selected ? drafts : draftCacheRef.current[item.objectId ?? String(index)]?.drafts}
                optimisticTextFieldIds={optimisticTextFieldIds}
                geometryOverrides={geometryOverrides}
                geometrySourceRects={geometrySourceRects}
              />
            </button>
            {canEdit ? (
              <span className="google-slide-rail-actions">
                <button type="button" title={`Duplicate slide ${index + 1}`} aria-label={`Duplicate slide ${index + 1}`} onClick={() => duplicateSlide(index)}>
                  <Copy size={11} />
                </button>
                <button className="danger" type="button" title={`Delete slide ${index + 1}`} aria-label={`Delete slide ${index + 1}`} disabled={visibleSlides.length <= 1} onClick={() => deleteSlide(index)}>
                  <Trash size={11} />
                </button>
              </span>
            ) : null}
          </div>
        ))}
      </nav> : null}
      <div className="google-slide-stage">
        <div
          ref={canvasRef}
          className="google-slide-canvas"
          style={{
            aspectRatio: pageAspectRatio,
            width: zoom === "fit" ? undefined : `${zoom}%`,
          }}
          onPointerDown={startMarquee}
        >
          <GoogleSlideVisual
            source={thumbnail}
            error={thumbnailError}
            slideNumber={selected + 1}
          />
          {canEdit ? sceneElements.map((element) => {
            if (shapeFields.some((field) => field.id === element.objectId)) return null;
            const rect = geometryOverrides[element.objectId] ?? googleSlideElementRect(element);
            return rect ? (
              <button
                className="google-slide-element-hit-target"
                type="button"
                key={element.objectId}
                aria-label={`Select ${element.kind}`}
                style={googleSlideRectStyle(rect)}
                onClick={() => setSelectedElementIds([element.objectId])}
                onPointerDown={(event) => startElementGesture(event, element, "move")}
              />
            ) : null;
          }) : null}
          {canEdit ? shapeFields.map((field) => (
            geometrySourceRects[field.id]
              ? <span
                  className="google-slide-text-origin-mask"
                  key={`${field.id}-origin`}
                  style={googleSlideRectStyle(geometrySourceRects[field.id])}
                  aria-hidden="true"
                />
              : null
          )) : null}
          {canEdit ? shapeFields.map((field) => (
            field.x != null && field.y != null && field.width != null && field.height != null ? (
              <GoogleSlideInlineEditor
                field={field}
                key={field.id}
                value={drafts[field.id] ?? ""}
                active={field.id === activeTextFieldId}
                optimistic={optimisticTextFieldIds.has(field.id) || geometryOverrides[field.id] != null}
                style={{
                  ...googleSlideRectStyle(geometryOverrides[field.id] ?? {
                    x: field.x,
                    y: field.y,
                    width: field.width,
                    height: field.height,
                  }),
                  textAlign: googleTextFormatCss(googleSlideTextFormat(field, 0)).textAlign,
                }}
                onFocus={(element) => {
                  activeSlideEditorRef.current = element;
                  setActiveTextFieldId(field.id);
                  setSelectedElementIds([field.id]);
                  setSlideFormat(formatOverrides[field.id] ?? googleSlideTextFormat(field, 0));
                  window.requestAnimationFrame(() => updateSlideSelection(field, element));
                }}
                onInput={(value) => {
                  if (value === (savedDraftsRef.current[field.id] ?? "")) clearOptimisticTextField(field.id);
                  else markSlideOptimistic(field.id);
                  setDrafts((current) => ({
                    ...current,
                    [field.id]: value,
                  }));
                }}
                onSelection={(element) => updateSlideSelection(field, element)}
                onBlur={(event) => {
                  if (event.relatedTarget instanceof HTMLElement && event.relatedTarget.closest("[data-google-format-toolbar]"))
                    return;
                  queueSlideText();
                  activeSlideEditorRef.current = null;
                  slideSelectionRangeRef.current = null;
                  setActiveTextFieldId(null);
                  setTextSelection({ start: 0, end: 0 });
                  setSlideFormat(null);
                }}
                onEscape={(element) => {
                  const saved = savedDraftsRef.current[field.id] ?? "";
                  element.textContent = saved;
                  setDrafts((current) => ({ ...current, [field.id]: saved }));
                  clearOptimisticTextField(field.id);
                }}
              />
            ) : null
          )) : null}
          {thumbnail ? sceneElements.map((element) => {
            const rect = geometryOverrides[element.objectId];
            const sourceRect = geometrySourceRects[element.objectId];
            return rect && sourceRect && !shapeFields.some((field) => field.id === element.objectId) ? (
              <div className="google-slide-element-proxy" style={googleSlideRectStyle(rect)} key={`${element.objectId}-proxy`} aria-hidden="true">
                <img src={thumbnail} alt="" style={googleSlideCropStyle(sourceRect)} />
              </div>
            ) : null;
          }) : null}
          {snapGuides.x != null ? <span className="google-slide-snap-guide vertical" style={{ left: `${snapGuides.x * 100}%` }} /> : null}
          {snapGuides.y != null ? <span className="google-slide-snap-guide horizontal" style={{ top: `${snapGuides.y * 100}%` }} /> : null}
          {marqueeRect ? <span className="google-slide-marquee" style={googleSlideRectStyle(marqueeRect)} aria-hidden="true" /> : null}
          {canEdit && selectedElements.length > 1 ? <>
            {selectedElementRects.map(({ element, rect }) => (
              <span className="google-slide-selection-frame multi" style={googleSlideRectStyle(rect)} key={element.objectId} aria-hidden="true" />
            ))}
            {selectedGroupRect ? (
              <div className="google-slide-selection-frame group" style={googleSlideRectStyle(selectedGroupRect)}>
                {(["top", "right", "bottom", "left"] as const).map((edge, index) => (
                  <button
                    className={`google-slide-drag-edge ${edge}`}
                    type="button"
                    key={edge}
                    tabIndex={index === 0 ? 0 : -1}
                    aria-hidden={index === 0 ? undefined : true}
                    aria-label={index === 0 ? `Move ${selectedElements.length} selected elements` : undefined}
                    onPointerDown={(event) => startGroupGesture(event, "move")}
                  />
                ))}
                {(["nw", "ne", "se", "sw"] as const).map((handle) => (
                  <button
                    className={`google-slide-resize-handle ${handle}`}
                    type="button"
                    key={handle}
                    aria-label={`Resize ${selectedElements.length} selected elements from ${handle}`}
                    onPointerDown={(event) => startGroupGesture(event, handle)}
                  />
                ))}
              </div>
            ) : null}
          </> : null}
          {canEdit && selectedElements.length === 1 && selectedElement && selectedElementRect ? (
            <div className="google-slide-selection-frame" style={googleSlideRectStyle(selectedElementRect)}>
              {(["top", "right", "bottom", "left"] as const).map((edge, index) => (
                <button
                  className={`google-slide-drag-edge ${edge}`}
                  type="button"
                  key={edge}
                  tabIndex={index === 0 ? 0 : -1}
                  aria-hidden={index === 0 ? undefined : true}
                  aria-label={index === 0 ? `Move selected ${selectedElement.kind}` : undefined}
                  onPointerDown={(event) => startElementGesture(event, selectedElement, "move")}
                />
              ))}
              {(["nw", "ne", "se", "sw"] as const).map((handle) => (
                <button
                  className={`google-slide-resize-handle ${handle}`}
                  type="button"
                  key={handle}
                  aria-label={`Resize selected ${selectedElement.kind} from ${handle}`}
                  onPointerDown={(event) => startElementGesture(event, selectedElement, handle)}
                />
              ))}
            </div>
          ) : null}
        </div>
        {thumbnailError ? <p className="sheet-hint error">{thumbnailError}</p> : null}
        {canEdit && shapeFields.some((field) => field.x == null || field.y == null || field.width == null || field.height == null) ? (
          <small className="google-slide-layout-hint">Some unsupported rotated text boxes remain view-only.</small>
        ) : null}
        {notesOpen ? (
          <label className="google-slide-inline-notes">
            <strong>Speaker notes</strong>
            {canEdit && notesField ? (
              <textarea
                aria-label="Edit speaker notes"
                value={drafts[notesField.id] ?? ""}
                onChange={(event) => {
                  markSlideOptimistic();
                  setDrafts((current) => ({
                    ...current,
                    [notesField.id]: event.currentTarget.value,
                  }));
                }}
                onBlur={queueSlideText}
              />
            ) : <p>{slide?.notes?.trim() || "No speaker notes on this slide."}</p>}
          </label>
        ) : null}
      </div>
      </div>
      {presenting && typeof document !== "undefined" ? createPortal(
        <div
          className="google-slides-presentation"
          role="dialog"
          aria-modal="true"
          aria-label={`Presenting slide ${selected + 1} of ${visibleSlides.length}`}
          tabIndex={-1}
          ref={presentationRef}
        >
          <button
            className="google-slides-presentation-exit"
            type="button"
            onClick={() => setPresenting(false)}
          >
            Exit <kbd>Esc</kbd>
          </button>
          <div className="google-slides-presentation-stage">
            <div
              className="google-slides-presentation-frame"
              style={{
                "--slide-aspect": pageAspectRatio,
                aspectRatio: pageAspectRatio,
              } as CSSProperties}
            >
              <GoogleSlideVisual
                source={thumbnail}
                error={thumbnailError}
                slideNumber={selected + 1}
              />
            </div>
          </div>
          <div className="google-slides-presentation-controls">
            <button type="button" disabled={selected === 0} onClick={() => selectRelative(-1)}>
              <ArrowLeft size={16} aria-hidden="true" /> Previous
            </button>
            <output aria-label={`Slide ${selected + 1} of ${visibleSlides.length}`} aria-live="polite">
              {selected + 1} / {visibleSlides.length}
            </output>
            <button type="button" disabled={selected === visibleSlides.length - 1} onClick={() => selectRelative(1)}>
              Next <ArrowRight size={16} aria-hidden="true" />
            </button>
          </div>
        </div>,
        document.body,
      ) : null}
    </div>
  );
}

type GoogleSlidesNavigationAction = "previous" | "next" | "first" | "last" | "exit";

export function googleSlidesNavigationAction(key: string): GoogleSlidesNavigationAction | null {
  if (key === "ArrowLeft" || key === "ArrowUp" || key === "PageUp") return "previous";
  if (key === "ArrowRight" || key === "ArrowDown" || key === "PageDown" || key === " " || key === "Spacebar") return "next";
  if (key === "Home") return "first";
  if (key === "End") return "last";
  if (key === "Escape") return "exit";
  return null;
}

export function googleSlideTextFormat(field: GoogleSlideEditableField, index: number): GoogleTextFormat {
  const runAt = (runs: GoogleSlideEditableField["styleRuns"]) => {
    const last = runs?.[runs.length - 1];
    return runs?.find((run) => run.start <= index && index < run.end)
      ?? (last && index >= last.end ? last : runs?.[0]);
  };
  const textStyle = runAt(field.styleRuns)?.style ?? {};
  const paragraphStyle = runAt(field.paragraphRuns)?.style ?? {};
  const weightedFont = isRecord(textStyle.weightedFontFamily) ? textStyle.weightedFontFamily : {};
  return {
    bold: textStyle.bold === true
      || (typeof weightedFont.weight === "number" && weightedFont.weight >= 600),
    italic: textStyle.italic === true,
    underline: textStyle.underline === true,
    fontSize: googleFontSize(textStyle.fontSize, 14),
    fontFamily: typeof weightedFont.fontFamily === "string"
      ? weightedFont.fontFamily
      : typeof textStyle.fontFamily === "string"
        ? textStyle.fontFamily
        : undefined,
    color: googleColorHex(textStyle.foregroundColor),
    alignment: googleTextAlignment(paragraphStyle.alignment),
  };
}

export function googleSlideTextSegments(field: GoogleSlideEditableField) {
  const boundaries = new Set([0, field.text.length]);
  for (const run of [...(field.styleRuns ?? []), ...(field.paragraphRuns ?? [])]) {
    boundaries.add(Math.max(0, Math.min(field.text.length, run.start)));
    boundaries.add(Math.max(0, Math.min(field.text.length, run.end)));
  }
  const offsets = [...boundaries].sort((left, right) => left - right);
  return offsets.slice(0, -1).flatMap((start, index) => {
    const end = offsets[index + 1];
    return end > start
      ? [{ start, end, text: field.text.slice(start, end), format: googleSlideTextFormat(field, start) }]
      : [];
  });
}

function GoogleSlideVisual({
  source,
  error,
  slideNumber,
}: {
  source: string | null;
  error: string | null;
  slideNumber: number;
}) {
  return source ? <img src={source} alt={`Slide ${slideNumber}`} /> : (
    <div
      className="google-slide-fallback"
      role="status"
      aria-label={error ? "Slide preview unavailable" : "Loading slide preview"}
      data-error={error ? "true" : undefined}
    >
      {!error ? <span className="spinner" aria-hidden="true" /> : null}
      {error ? <small>Preview unavailable</small> : null}
    </div>
  );
}

function SlideRailThumbnail({
  fileId,
  objectId,
  active,
  optimistic,
  revision,
  selectedSource,
  slide,
  drafts,
  optimisticTextFieldIds,
  geometryOverrides,
  geometrySourceRects,
}: {
  fileId: string;
  objectId?: string | null;
  active: boolean;
  optimistic?: boolean;
  revision: number;
  selectedSource: string | null;
  slide: LocalGoogleSlide;
  drafts?: Record<string, string>;
  optimisticTextFieldIds: Set<string>;
  geometryOverrides: Record<string, GoogleSlideRect>;
  geometrySourceRects: Record<string, GoogleSlideRect>;
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
    if (!visible || !objectId || optimistic || selectedSource) return;
    const requestKey = googleSlideThumbnailRequestKey(fileId, objectId, revision);
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
  }, [active, fileId, objectId, optimistic, revision, selectedSource, visible]);

  useEffect(() => () => {
    if (objectUrlRef.current) URL.revokeObjectURL(objectUrlRef.current);
  }, []);

  const thumbnailSource = selectedSource || source;
  const textIds = new Set(slide.textElements?.map((field) => field.objectId));
  return (
    <span className="google-slide-rail-thumbnail" ref={rootRef}>
      {thumbnailSource ? <img src={thumbnailSource} alt="" /> : <span className="google-slide-rail-placeholder" aria-hidden="true" />}
      {slide.textElements?.map((field, index) => {
        const rect = geometryOverrides[field.objectId] ?? (
          field.x == null || field.y == null || field.width == null || field.height == null
            ? null
            : { x: field.x, y: field.y, width: field.width, height: field.height }
        );
        if (!rect || (!optimisticTextFieldIds.has(field.objectId) && !geometryOverrides[field.objectId])) return null;
        const previewField: GoogleSlideEditableField = {
          ...field,
          id: field.objectId,
          label: `Text ${index + 1}`,
          kind: "shape",
          text: drafts?.[field.objectId] ?? field.text,
        };
        return <Fragment key={`${field.objectId}-optimistic-thumbnail`}>
          {geometrySourceRects[field.objectId] ? (
            <span className="google-slide-rail-origin-mask" style={googleSlideRectStyle(geometrySourceRects[field.objectId])} />
          ) : null}
          <span
            className="google-slide-rail-text-preview"
            data-content-alignment={field.contentAlignment ?? undefined}
            style={{
              ...googleSlideRectStyle(rect),
              textAlign: googleTextFormatCss(googleSlideTextFormat(previewField, 0)).textAlign,
            }}
          >
            {googleSlideTextSegments(previewField).map((segment) => (
              <span
                key={`${segment.start}-${segment.end}`}
                style={{
                  color: segment.format.color,
                  fontSize: `${segment.format.fontSize * (field.fontScale ?? 1) / 7.2}cqw`,
                  fontFamily: segment.format.fontFamily,
                  fontWeight: segment.format.bold ? "700" : undefined,
                  fontStyle: segment.format.italic ? "italic" : undefined,
                  textDecoration: segment.format.underline ? "underline" : undefined,
                }}
              >
                {segment.text}
              </span>
            ))}
          </span>
        </Fragment>;
      })}
      {thumbnailSource ? slide.elements?.map((element) => {
        const rect = geometryOverrides[element.objectId];
        const sourceRect = geometrySourceRects[element.objectId];
        return rect && sourceRect && !textIds.has(element.objectId) ? (
          <Fragment key={`${element.objectId}-optimistic-thumbnail`}>
            <span className="google-slide-rail-origin-mask" style={googleSlideRectStyle(sourceRect)} />
            <span className="google-slide-rail-element-proxy" style={googleSlideRectStyle(rect)}>
              <img src={thumbnailSource} alt="" style={googleSlideCropStyle(sourceRect)} />
            </span>
          </Fragment>
        ) : null;
      }) : null}
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

function googleDocNamedStyleMap(document: Record<string, unknown>): GoogleDocNamedStyleMap {
  const namedStyles = isRecord(document.namedStyles) && Array.isArray(document.namedStyles.styles)
    ? document.namedStyles.styles
    : [];
  return Object.fromEntries(namedStyles.flatMap((value) => {
    if (!isRecord(value) || typeof value.namedStyleType !== "string") return [];
    return [[value.namedStyleType, {
      paragraphStyle: isRecord(value.paragraphStyle) ? value.paragraphStyle : {},
      textStyle: isRecord(value.textStyle) ? value.textStyle : {},
    }]];
  }));
}

function googleDocParagraphFormat(
  paragraph: Record<string, unknown>,
  paragraphStyle: Record<string, unknown>,
  inheritedStyle: Record<string, unknown>,
  namedStyleType: string,
  lists: Record<string, unknown>,
): GoogleDocParagraphFormat {
  const elements = Array.isArray(paragraph.elements) ? paragraph.elements : [];
  const runStyles = elements.flatMap((element) => {
    if (!isRecord(element) || !isRecord(element.textRun)) return [];
    const direct = isRecord(element.textRun.textStyle) ? element.textRun.textStyle : {};
    return [{ ...inheritedStyle, ...direct }];
  });
  const enabledForAllRuns = (field: "bold" | "italic" | "underline") =>
    (runStyles.length ? runStyles : [inheritedStyle]).every((style) => style[field] === true);
  const bullet = isRecord(paragraph.bullet) ? paragraph.bullet : null;
  const firstStyle = runStyles[0] ?? inheritedStyle;
  return {
    namedStyleType,
    bold: enabledForAllRuns("bold"),
    italic: enabledForAllRuns("italic"),
    underline: enabledForAllRuns("underline"),
    fontSize: googleFontSize(firstStyle.fontSize, 11),
    color: googleColorHex(firstStyle.foregroundColor),
    alignment: googleTextAlignment(paragraphStyle.alignment),
    listKind: bullet ? googleDocListKind(bullet, lists) : null,
  };
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
    paddingInlineStart: googleDocDimension(style.indentStart),
    paddingInlineEnd: googleDocDimension(style.indentEnd),
  };
}

function googleDocTextStyle(style: Record<string, unknown>): CSSProperties {
  let fontSize = googleDocDimension(style.fontSize);
  const weightedFont = isRecord(style.weightedFontFamily) ? style.weightedFontFamily : {};
  const fontFamily = typeof weightedFont.fontFamily === "string" ? weightedFont.fontFamily : undefined;
  const weight = typeof weightedFont.weight === "number" ? weightedFont.weight : undefined;
  const decorations = [
    style.underline === true ? "underline" : "",
    style.strikethrough === true ? "line-through" : "",
  ].filter(Boolean).join(" ");
  const baseline = style.baselineOffset === "SUPERSCRIPT"
    ? "super"
    : style.baselineOffset === "SUBSCRIPT"
      ? "sub"
      : undefined;
  if (baseline) fontSize = fontSize ? `calc(${fontSize} * .83)` : "0.83em";
  return {
    color: googleDocColor(style.foregroundColor),
    backgroundColor: googleDocColor(style.backgroundColor),
    fontFamily: fontFamily ? `${fontFamily}, Arial, sans-serif` : undefined,
    fontSize,
    fontWeight: style.bold === true ? Math.max(700, weight ?? 400) : weight,
    fontStyle: style.italic === true ? "italic" : undefined,
    fontVariantCaps: style.smallCaps === true ? "small-caps" : undefined,
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

function googleColorHex(value: unknown): string {
  const choice = isRecord(value) && isRecord(value.color)
    ? value.color
    : isRecord(value) && isRecord(value.opaqueColor)
      ? value.opaqueColor
      : value;
  const rgb = isRecord(choice) && isRecord(choice.rgbColor) ? choice.rgbColor : choice;
  if (!isRecord(rgb)) return "#000000";
  const channel = (key: string) => Math.round(
    Math.min(1, Math.max(0, typeof rgb[key] === "number" ? rgb[key] : 0)) * 255,
  ).toString(16).padStart(2, "0");
  return `#${channel("red")}${channel("green")}${channel("blue")}`;
}

function googleFontSize(value: unknown, fallback: number): number {
  return isRecord(value) && typeof value.magnitude === "number"
    ? Math.round(value.magnitude)
    : fallback;
}

function googleTextAlignment(value: unknown): GoogleTextAlignment {
  return value === "CENTER" || value === "END" || value === "JUSTIFIED" ? value : "START";
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

export function googleFileKindDetail(file: GoogleFileSummary): string {
  const label = googleFileKindLabel(file);
  return file.mime_type === "application/vnd.google-apps.document" && file.capabilities.can_edit
    ? `${label} · Click text to edit`
    : label;
}

export function googleDocFitScale(width: number, paddingLeft: number, paddingRight: number): number {
  return Math.max(0.1, (width - paddingLeft - paddingRight) / 816);
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
