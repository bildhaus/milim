import {
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
  googleDocEditableParagraph,
  googleDocTextReplacement,
  googleSheetCellRange,
  googleWorkspaceFileUrl,
  parseGoogleSheetClipboard,
  type GoogleDocEditableParagraph,
} from "../lib/googleWorkspace";
import { useContextMenu } from "./ContextMenu";
import { ArrowLeft, ArrowRight, Copy, ExternalLink, FileText, Folder, MoreHorizontal, Plus, Refresh, Search, Sidebar, Trash } from "./icons";

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

type AutosaveStatus = "idle" | "saving" | "saved" | "error";

function useAutosave(
  dirty: boolean,
  changeKey: string,
  save: () => Promise<void>,
  resetKey: string,
) {
  const saveRef = useRef(save);
  const dirtyRef = useRef(dirty);
  const changeKeyRef = useRef(changeKey);
  const savingRef = useRef(false);
  const queuedRef = useRef(false);
  const failedKeyRef = useRef<string | null>(null);
  const [status, setStatus] = useState<AutosaveStatus>("idle");
  const [error, setError] = useState<string | null>(null);
  saveRef.current = save;
  dirtyRef.current = dirty;
  changeKeyRef.current = changeKey;

  const flush = useCallback(async () => {
    if (!dirtyRef.current) return true;
    if (savingRef.current) {
      queuedRef.current = true;
      return false;
    }
    savingRef.current = true;
    const savingKey = changeKeyRef.current;
    setStatus("saving");
    setError(null);
    try {
      await saveRef.current();
      failedKeyRef.current = null;
      setStatus("saved");
      return true;
    } catch (cause) {
      failedKeyRef.current = savingKey;
      setStatus("error");
      setError(cause instanceof Error ? cause.message : String(cause));
      return false;
    } finally {
      savingRef.current = false;
      if (queuedRef.current && dirtyRef.current) {
        queuedRef.current = false;
        window.setTimeout(() => void flush(), 0);
      }
    }
  }, []);

  useEffect(() => {
    failedKeyRef.current = null;
    setStatus("idle");
    setError(null);
  }, [resetKey]);

  useEffect(() => {
    if (!dirty || failedKeyRef.current === changeKey) return;
    const timeout = window.setTimeout(() => void flush(), 800);
    return () => window.clearTimeout(timeout);
  }, [changeKey, dirty, flush]);

  return { error, flush, saving: status === "saving", status };
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
  const width = Math.max(
    1,
    ...preview.values.map((row) => row.length),
    ...preview.formulas.map((row) => row.length),
  );
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
  const [operationSaving, setOperationSaving] = useState(false);
  const saving = operationSaving;
  const [editError, setEditError] = useState<string | null>(null);
  const { openMenuAt } = useContextMenu();
  const activeSheet = sheetTitleFromRange(preview.range);
  const activeSheetId = sheets.find((sheet) => sheet.title === activeSheet)?.id ?? null;
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

  async function applyEdit(
    operations: Parameters<typeof editGoogleSheet>[1],
  ) {
    setOperationSaving(true);
    setEditError(null);
    try {
      await editGoogleSheet(preview.file.id, operations);
      onSaved();
    } catch (cause) {
      setEditError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setOperationSaving(false);
    }
  }

  function startCellEdit(row: number, column: number, source: "cell" | "formula" = "cell") {
    if (!preview.file.capabilities.can_edit || operationSaving || cellAutosave.saving) return;
    editSourceRef.current = source;
    if (editingCell?.[0] === row && editingCell[1] === column) return;
    const value = displayCell(preview.values[row]?.[column]);
    const formula = displayCell(preview.formulas[row]?.[column]);
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
  const cellAutosave = useAutosave(
    Boolean(editingCell) && editValue !== savedEditValue,
    `${editingAddress}\0${editValue}`,
    async () => {
      if (!editingCell) return;
      const address = `${columnLabel(rangeOrigin.column + editingCell[1])}${rangeOrigin.row + editingCell[0]}`;
      const snapshot = editValue;
      await editGoogleSheet(preview.file.id, [{
        action: "set_values",
        range: googleSheetCellRange(activeSheet, address),
        values: [[snapshot]],
        input_option: "USER_ENTERED",
      }]);
      setSavedEditValue(snapshot);
      onSaved();
    },
    `${preview.file.id}\0${preview.range}\0${editingAddress}`,
  );

  async function finishCellEdit(moveDown = false, focusGrid = false) {
    if (!editingCell) return;
    const [row, column] = editingCell;
    if (editValue !== savedEditValue && !await cellAutosave.flush()) return;
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
    if (!preview.file.capabilities.can_edit || operationSaving) return;
    const values = parseGoogleSheetClipboard(event.clipboardData.getData("text/plain"));
    const cells = values.reduce((count, row) => count + row.length, 0);
    if (!cells) return;
    event.preventDefault();
    if (cells > 5_000) {
      setEditError("Paste is limited to 5,000 cells.");
      return;
    }
    void applyEdit([{
      action: "set_values",
      range: googleSheetCellRange(activeSheet, activeAddress),
      values,
      input_option: "USER_ENTERED",
    }]);
  }

  function editDimension(
    action: "insert_rows" | "delete_rows" | "insert_columns" | "delete_columns",
  ) {
    if (activeSheetId === null || operationSaving) return;
    const deleting = action.startsWith("delete");
    const row = action.endsWith("rows");
    const selectedIndex = row
      ? rangeOrigin.row - 1 + selectedRow
      : rangeOrigin.column + selectedColumn;
    if (deleting && !window.confirm(`Delete ${row ? "row" : "column"} ${row ? rangeOrigin.row + selectedRow : columnLabel(rangeOrigin.column + selectedColumn)}?`)) {
      return;
    }
    const start = deleting ? selectedIndex : selectedIndex + 1;
    void applyEdit([{
      action,
      sheet_id: activeSheetId,
      start,
      end: start + 1,
    }]);
  }

  function openDimensionMenu(event: ReactMouseEvent<HTMLButtonElement>) {
    const trigger = event.currentTarget;
    const rect = trigger.getBoundingClientRect();
    const disabled = activeSheetId === null || saving || !preview.file.capabilities.can_edit;
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
          disabled={activeSheetId === null || saving || !preview.file.capabilities.can_edit}
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
        <small>{preview.file.capabilities.can_edit
          ? cellAutosave.status === "saving" ? "Saving..." : cellAutosave.status === "saved" ? "Saved" : "Editable"
          : "View only"}</small>
      </div>
      {editError || cellAutosave.error ? <div className="google-sheet-edit-error" role="alert">{editError || cellAutosave.error}</div> : null}
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
            {preview.values.length ? preview.values.map((row, rowIndex) => (
              <tr key={rowIndex}>
                <th>{rangeOrigin.row + rowIndex}</th>
                {Array.from({ length: width }, (_, columnIndex) => {
                  const formula = displayCell(preview.formulas[rowIndex]?.[columnIndex]);
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
  color: string;
  alignment: GoogleTextAlignment;
};

type GoogleTextFormatChange =
  | { field: "bold" | "italic" | "underline"; value: boolean }
  | { field: "fontSize"; value: number }
  | { field: "color"; value: string }
  | { field: "alignment"; value: GoogleTextAlignment };

export function GoogleTextFormatToolbar({
  format,
  disabled,
  before,
  after,
  className = "",
  onChange,
}: {
  format: GoogleTextFormat;
  disabled?: boolean;
  before?: ReactNode;
  after?: ReactNode;
  className?: string;
  onChange: (change: GoogleTextFormatChange) => void;
}) {
  return (
    <span
      className={`google-text-format-toolbar ${className}`.trim()}
      role="toolbar"
      aria-label="Text formatting"
      data-google-format-toolbar="true"
    >
      {before}
      {(["bold", "italic", "underline"] as const).map((field) => (
        <button
          type="button"
          key={field}
          className={format[field] ? "active" : ""}
          aria-label={field[0].toUpperCase() + field.slice(1)}
          aria-pressed={format[field]}
          disabled={disabled}
          onClick={() => onChange({ field, value: !format[field] })}
        >
          {field === "bold" ? <strong>B</strong> : field === "italic" ? <em>I</em> : <u>U</u>}
        </button>
      ))}
      <label>
        <span className="sr-only">Font size</span>
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
      <label>
        <span className="sr-only">Text alignment</span>
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
        <span className="sr-only">Text color</span>
        <input
          type="color"
          aria-label="Text color"
          value={format.color}
          disabled={disabled}
          onChange={(event) => onChange({ field: "color", value: event.currentTarget.value })}
        />
      </label>
      {after}
    </span>
  );
}

type GoogleDocParagraphFormat = GoogleTextFormat & {
  namedStyleType: string;
  listKind: "ordered" | "unordered" | null;
};

type GoogleDocEditorState = GoogleDocEditableParagraph & GoogleDocParagraphFormat & {
  blockIndex: number;
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
  const [formatStatus, setFormatStatus] = useState<AutosaveStatus>("idle");
  const [formatError, setFormatError] = useState<string | null>(null);
  const contentRef = useRef<HTMLDivElement | null>(null);
  const body = isRecord(document.body) && Array.isArray(document.body.content)
    ? document.body.content
    : [];
  const inlineObjects = isRecord(document.inlineObjects)
    ? document.inlineObjects
    : {};
  const lists = isRecord(document.lists) ? document.lists : {};
  const namedStyles = useMemo(() => googleDocNamedStyleMap(document), [document]);
  const outline = useMemo(() => googleDocOutline(body), [body]);
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

  function editParagraph(paragraph: GoogleDocEditableParagraph, blockIndex: number, format: GoogleDocParagraphFormat) {
    if (!canEdit || docAutosave.saving) return;
    setEditing({ ...paragraph, blockIndex, ...format });
    setEditorText(paragraph.text);
    setSavedEditorText(paragraph.text);
    setEditorSelection({ start: 0, end: 0 });
    setFormatStatus("idle");
    setFormatError(null);
  }

  const docAutosave = useAutosave(
    Boolean(editing) && editorText !== savedEditorText,
    `${editing?.blockIndex ?? ""}\0${editorText}`,
    async () => {
      if (!editing) return;
      const snapshot = editorText;
      const replacement = googleDocTextReplacement(editing.start, savedEditorText, snapshot);
      if (!replacement) return;
      const operations: Parameters<typeof editGoogleDoc>[1] = [];
      if (replacement.end > replacement.start) {
        operations.push({ action: "delete_range", start: replacement.start, end: replacement.end });
      }
      if (replacement.text) {
        operations.push({ action: "insert_text", index: replacement.start, text: replacement.text });
      }
      await editGoogleDoc(fileId, operations);
      setSavedEditorText(snapshot);
      setEditing((current) => current ? {
        ...current,
        end: current.start + snapshot.length,
        text: snapshot,
      } : current);
      onSaved();
    },
    `${fileId}\0${editing?.blockIndex ?? ""}`,
  );

  async function applyParagraphFormat(
    operations: Parameters<typeof editGoogleDoc>[1],
    patch: Partial<GoogleDocParagraphFormat>,
  ) {
    if (!editing || docAutosave.saving || formatStatus === "saving") return;
    if (editorText !== savedEditorText && !await docAutosave.flush()) return;
    setFormatStatus("saving");
    setFormatError(null);
    try {
      await editGoogleDoc(fileId, operations);
      setEditing((current) => current ? { ...current, ...patch } : current);
      setFormatStatus("saved");
      onSaved();
    } catch (cause) {
      setFormatStatus("error");
      setFormatError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  function editorBlur(event: FocusEvent<HTMLElement>) {
    if (
      event.relatedTarget instanceof Node
      && (event.currentTarget.contains(event.relatedTarget)
        || event.relatedTarget instanceof HTMLElement && event.relatedTarget.closest("[data-google-format-toolbar]"))
    ) return;
    if (editorBusy) return;
    if (editorText === savedEditorText) setEditing(null);
    else void docAutosave.flush().then((saved) => {
      if (saved) setEditing(null);
    });
  }

  const selectedEditorRange = editorSelection.end > editorSelection.start
    ? editorSelection
    : { start: 0, end: editorText.length };
  const editorRange = editing && selectedEditorRange.end > selectedEditorRange.start
    ? {
      start: editing.start + selectedEditorRange.start,
      end: editing.start + selectedEditorRange.end,
    }
    : null;
  const editorBusy = docAutosave.saving || formatStatus === "saving";

  function applyDocumentTextFormat(change: GoogleTextFormatChange) {
    if (!editorRange) return;
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
            {body.length ? body.map((item, index) => editing?.blockIndex === index ? (
              <span className="google-doc-inline-editor" onBlur={editorBlur} key={index}>
                <GoogleTextFormatToolbar
                  className="google-doc-format-toolbar"
                  format={editing}
                  disabled={editorBusy || !editorRange}
                  onChange={applyDocumentTextFormat}
                  before={(
                    <select
                      aria-label="Paragraph style"
                      value={editing.namedStyleType}
                      disabled={editorBusy || !editorRange}
                      onChange={(event) => {
                        const namedStyleType = event.currentTarget.value;
                        if (!editorRange) return;
                        void applyParagraphFormat(
                          [{ action: "set_paragraph_style", ...editorRange, named_style_type: namedStyleType }],
                          { namedStyleType },
                        );
                      }}
                    >
                      <option value="NORMAL_TEXT">Normal text</option>
                      <option value="TITLE">Title</option>
                      <option value="SUBTITLE">Subtitle</option>
                      {[1, 2, 3, 4, 5, 6].map((level) => (
                        <option value={`HEADING_${level}`} key={level}>Heading {level}</option>
                      ))}
                    </select>
                  )}
                  after={(
                    <>
                      {(["unordered", "ordered"] as const).map((kind) => (
                        <button
                          type="button"
                          key={kind}
                          className={editing.listKind === kind ? "active" : ""}
                          aria-label={kind === "unordered" ? "Bulleted list" : "Numbered list"}
                          aria-pressed={editing.listKind === kind}
                          disabled={editorBusy || !editorRange}
                          onClick={() => {
                            if (!editorRange) return;
                            const active = editing.listKind === kind;
                            void applyParagraphFormat(
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
                          }}
                        >
                          {kind === "unordered" ? "Bullets" : "Numbering"}
                        </button>
                      ))}
                      <button
                        type="button"
                        aria-label="Clear formatting"
                        disabled={editorBusy || !editorRange}
                        onClick={() => {
                          if (!editorRange) return;
                          const operations: Parameters<typeof editGoogleDoc>[1] = [
                            { action: "clear_text_style", ...editorRange },
                            { action: "set_paragraph_style", ...editorRange, named_style_type: "NORMAL_TEXT", alignment: "START" },
                          ];
                          if (editing.listKind) operations.push({ action: "delete_bullets", ...editorRange });
                          void applyParagraphFormat(operations, {
                            namedStyleType: "NORMAL_TEXT",
                            bold: false,
                            italic: false,
                            underline: false,
                            fontSize: 11,
                            color: "#000000",
                            alignment: "START",
                            listKind: null,
                          });
                        }}
                      >
                        Clear
                      </button>
                    </>
                  )}
                />
                <textarea
                  aria-label="Edit document paragraph"
                  value={editorText}
                  autoFocus
                  style={{
                    color: editing.color,
                    fontSize: `calc(${editing.fontSize}pt * var(--doc-zoom))`,
                    fontWeight: editing.bold ? 700 : undefined,
                    fontStyle: editing.italic ? "italic" : undefined,
                    textDecoration: editing.underline ? "underline" : undefined,
                    textAlign: editing.alignment === "JUSTIFIED"
                      ? "justify"
                      : editing.alignment.toLocaleLowerCase() as CSSProperties["textAlign"],
                  }}
                  onChange={(event) => setEditorText(event.currentTarget.value)}
                  onSelect={(event) => setEditorSelection({
                    start: event.currentTarget.selectionStart,
                    end: event.currentTarget.selectionEnd,
                  })}
                  onKeyDown={(event) => {
                    if (event.key === "Escape") {
                      setEditorText(savedEditorText);
                      setEditing(null);
                    }
                  }}
                />
                <small aria-live="polite">
                  {editorBusy ? "Saving..." : docAutosave.status === "saved" || formatStatus === "saved" ? "Saved" : ""}
                </small>
                {docAutosave.error || formatError
                  ? <small className="error" role="alert">{docAutosave.error || formatError}</small>
                  : null}
              </span>
            ) : (
              <DocStructuralElement
                value={item}
                inlineObjects={inlineObjects}
                lists={lists}
                namedStyles={namedStyles}
                query={normalizedQuery}
                blockIndex={index}
                onEditParagraph={canEdit ? editParagraph : undefined}
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
  namedStyles,
  query,
  blockIndex,
  onEditParagraph,
}: {
  value: unknown;
  inlineObjects: Record<string, unknown>;
  lists: Record<string, unknown>;
  namedStyles: GoogleDocNamedStyleMap;
  query: string;
  blockIndex?: number;
  onEditParagraph?: (paragraph: GoogleDocEditableParagraph, blockIndex: number, format: GoogleDocParagraphFormat) => void;
}): ReactNode {
  if (!isRecord(value)) return null;
  if (isRecord(value.paragraph) && Array.isArray(value.paragraph.elements)) {
    const directStyle = isRecord(value.paragraph.paragraphStyle) ? value.paragraph.paragraphStyle : {};
    const namedStyle = typeof directStyle.namedStyleType === "string" ? directStyle.namedStyleType : "NORMAL_TEXT";
    const inherited = namedStyles[namedStyle] ?? namedStyles.NORMAL_TEXT;
    const style = { ...(namedStyles.NORMAL_TEXT?.paragraphStyle ?? {}), ...(inherited?.paragraphStyle ?? {}), ...directStyle };
    const textStyle = { ...(namedStyles.NORMAL_TEXT?.textStyle ?? {}), ...(inherited?.textStyle ?? {}) };
    const content = value.paragraph.elements.map((element, index) => (
      <DocParagraphElement value={element} inlineObjects={inlineObjects} inheritedStyle={textStyle} query={query} key={index} />
    ));
    const paragraphStyle = googleDocParagraphStyle(style);
    const headingProps = blockIndex === undefined ? {} : { "data-doc-heading": blockIndex };
    let paragraphNode: ReactNode;
    if (namedStyle === "TITLE") {
      paragraphNode = <h1 style={paragraphStyle} {...headingProps}>{content}</h1>;
    } else if (namedStyle === "SUBTITLE") {
      paragraphNode = <p className="google-doc-subtitle" style={paragraphStyle}>{content}</p>;
    } else if (namedStyle.startsWith("HEADING_")) {
      const level = Math.min(6, Math.max(2, Number(namedStyle.slice(-1)) + 1));
      if (level === 2) paragraphNode = <h2 style={paragraphStyle} {...headingProps}>{content}</h2>;
      else if (level === 3) paragraphNode = <h3 style={paragraphStyle} {...headingProps}>{content}</h3>;
      else if (level === 4) paragraphNode = <h4 style={paragraphStyle} {...headingProps}>{content}</h4>;
      else if (level === 5) paragraphNode = <h5 style={paragraphStyle} {...headingProps}>{content}</h5>;
      else paragraphNode = <h6 style={paragraphStyle} {...headingProps}>{content}</h6>;
    } else if (isRecord(value.paragraph.bullet)) {
      const bullet = value.paragraph.bullet;
      const nestingLevel = typeof bullet.nestingLevel === "number" ? bullet.nestingLevel : 0;
      paragraphNode = (
        <p
          className="google-doc-list-item"
          data-list-kind={googleDocListKind(bullet, lists)}
          style={{ ...paragraphStyle, paddingInlineStart: `${18 + nestingLevel * 18}px` }}
        >
          {content}
        </p>
      );
    } else {
      paragraphNode = <p style={paragraphStyle}>{content}</p>;
    }
    const edit = onEditParagraph;
    const editable = edit ? googleDocEditableParagraph(value) : null;
    return editable && edit && blockIndex !== undefined ? (
      <div
        className="google-doc-editable-block"
        role="button"
        tabIndex={0}
        title="Double-click to edit this paragraph"
        onDoubleClick={() => edit(editable, blockIndex, googleDocParagraphFormat(value.paragraph, style, textStyle, namedStyle, lists))}
        onKeyDown={(event) => {
          if (event.key === "Enter") edit(editable, blockIndex, googleDocParagraphFormat(value.paragraph, style, textStyle, namedStyle, lists));
        }}
      >
        {paragraphNode}
      </div>
    ) : paragraphNode;
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
}: {
  value: unknown;
  inlineObjects: Record<string, unknown>;
  inheritedStyle: Record<string, unknown>;
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
  const directStyle = isRecord(value.textRun.textStyle) ? value.textRun.textStyle : {};
  const style = { ...inheritedStyle, ...directStyle };
  const textStyle = googleDocTextStyle(style);
  return (
    <span className={isRecord(style.link) ? "google-doc-link" : undefined} style={textStyle}>
      {highlightGoogleDocText(value.textRun.content, query)}
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
  x?: number | null;
  y?: number | null;
  width?: number | null;
  height?: number | null;
};

export function SlidesPreview({
  fileId,
  slides,
  pageAspectRatio,
  active,
  canEdit,
  onSaved,
}: {
  fileId: string;
  slides: Extract<GoogleFilePreview, { kind: "presentation" }>["slides"];
  pageAspectRatio: number;
  active: boolean;
  canEdit: boolean;
  onSaved: () => void;
}) {
  const [selected, setSelected] = useState(0);
  const slide = slides[selected] ?? slides[0];
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
  const [editorOpen, setEditorOpen] = useState(false);
  const [presenting, setPresenting] = useState(false);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [savedDrafts, setSavedDrafts] = useState<Record<string, string>>({});
  const [activeTextFieldId, setActiveTextFieldId] = useState<string | null>(null);
  const [textSelection, setTextSelection] = useState({ start: 0, end: 0 });
  const [slideFormat, setSlideFormat] = useState<GoogleTextFormat | null>(null);
  const [slideFormatStatus, setSlideFormatStatus] = useState<AutosaveStatus>("idle");
  const [slideFormatError, setSlideFormatError] = useState<string | null>(null);
  const draftSlideRef = useRef<string | null>(null);
  const savedDraftsRef = useRef<Record<string, string>>({});
  const editableFields = useMemo(() => {
    if (!slide) return [];
    const fields: GoogleSlideEditableField[] = (slide.textElements ?? []).map((element, index) => ({
      id: element.objectId,
      label: `Text ${index + 1}`,
      text: element.text,
      kind: "shape" as const,
      styleRuns: element.styleRuns,
      paragraphRuns: element.paragraphRuns,
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
    return slides.flatMap((item, index) => `${item.text}\n${item.notes ?? ""}`.toLocaleLowerCase().includes(normalized) ? [index] : []);
  }, [query, slides]);

  useEffect(() => {
    if (selected < slides.length) return;
    setSelected(Math.max(0, slides.length - 1));
  }, [selected, slides.length]);

  useEffect(() => {
    const next = Object.fromEntries(editableFields.map((field) => [field.id, field.text]));
    const slideKey = slide?.objectId ?? String(selected);
    if (draftSlideRef.current !== slideKey) {
      draftSlideRef.current = slideKey;
      setDrafts(next);
      setSavedDrafts(next);
      setActiveTextFieldId(null);
      setSlideFormat(null);
      setSlideFormatError(null);
      savedDraftsRef.current = next;
      return;
    }
    setDrafts((current) => Object.keys(current).some((id) => current[id] !== savedDraftsRef.current[id])
      ? current
      : next);
    setSavedDrafts(next);
    savedDraftsRef.current = next;
  }, [editableFields, selected, slide?.objectId]);

  useEffect(() => {
    thumbnailRequestRef.current = null;
    setThumbnail(null);
  }, [slides]);

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

  function selectRelative(delta: -1 | 1) {
    setSelected((current) => Math.min(slides.length - 1, Math.max(0, current + delta)));
  }

  const applyNavigationAction = useCallback((action: GoogleSlidesNavigationAction) => {
    if (action === "previous")
      setSelected((current) => Math.max(0, current - 1));
    else if (action === "next")
      setSelected((current) => Math.min(slides.length - 1, current + 1));
    else if (action === "first") setSelected(0);
    else if (action === "last") setSelected(slides.length - 1);
  }, [slides.length]);

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
    setSelected(matchingSlides[next]);
  }

  const hasSlideChanges = editableFields.some((field) => (drafts[field.id] ?? "") !== (savedDrafts[field.id] ?? ""));
  const slideAutosave = useAutosave(
    hasSlideChanges,
    JSON.stringify(drafts),
    async () => {
      const operations: GoogleSlidesEditOperation[] = [];
      const snapshot = { ...drafts };
      const savingSlideKey = slide?.objectId ?? String(selected);
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
      if (!operations.length) return;
      await editGoogleSlides(fileId, operations);
      if (draftSlideRef.current === savingSlideKey) {
        setSavedDrafts(snapshot);
        savedDraftsRef.current = snapshot;
      }
      onSaved();
    },
    `${fileId}\0${slide?.objectId ?? selected}`,
  );
  const shapeFields = editableFields.filter((field) => field.kind === "shape");
  const notesField = editableFields.find((field) => field.kind === "notes");
  const activeTextField = shapeFields.find((field) => field.id === activeTextFieldId);
  const activeText = activeTextField ? drafts[activeTextField.id] ?? "" : "";
  const selectedTextRange = textSelection.end > textSelection.start
    ? textSelection
    : { start: 0, end: activeText.length };
  const activeTextRange = activeTextField && selectedTextRange.end > selectedTextRange.start
    ? selectedTextRange
    : null;
  const activeTextFormat = slideFormat
    ?? (activeTextField ? googleSlideTextFormat(activeTextField, textSelection.start) : null);
  const slideEditorBusy = slideAutosave.saving || slideFormatStatus === "saving";

  async function applySlideTextFormat(change: GoogleTextFormatChange) {
    if (!activeTextField || !activeTextRange || !activeTextFormat || slideEditorBusy) return;
    if (hasSlideChanges && !await slideAutosave.flush()) return;
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
    setSlideFormatStatus("saving");
    setSlideFormatError(null);
    try {
      await editGoogleSlides(fileId, [operation]);
      setSlideFormat({ ...activeTextFormat, [change.field]: change.value });
      setSlideFormatStatus("saved");
      onSaved();
    } catch (cause) {
      setSlideFormatStatus("error");
      setSlideFormatError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  if (!slides.length) return <div className="google-workspace-state"><strong>This presentation is empty.</strong></div>;

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
      <div className="google-viewer-toolbar">
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
        {canEdit && editableFields.length ? (
          <button
            className={`google-viewer-text-button${editorOpen ? " active" : ""}`}
            type="button"
            aria-pressed={editorOpen}
            onClick={() => setEditorOpen((value) => !value)}
          >
            Edit slide text
          </button>
        ) : null}
        {slides.some((item) => item.notesObjectId || item.notes?.trim()) ? (
          <button className={`google-viewer-text-button${notesOpen ? " active" : ""}`} type="button" aria-pressed={notesOpen} onClick={() => setNotesOpen((value) => !value)}>
            Notes
          </button>
        ) : null}
        {slideAutosave.status === "saving" || slideAutosave.status === "saved" || slideFormatStatus === "saving" || slideFormatStatus === "saved" ? (
          <small className="google-autosave-status" aria-live="polite">
            {slideAutosave.status === "saving" || slideFormatStatus === "saving" ? "Saving..." : "Saved"}
          </small>
        ) : null}
        <button
          className="google-viewer-present-button"
          type="button"
          aria-haspopup="dialog"
          onClick={(event) => {
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
      </nav> : null}
      <div className="google-slide-stage">
        {editorOpen && activeTextFormat ? (
          <GoogleTextFormatToolbar
            className="google-slide-format-toolbar"
            format={activeTextFormat}
            disabled={slideEditorBusy || !activeTextRange}
            onChange={(change) => void applySlideTextFormat(change)}
          />
        ) : null}
        <div
          className="google-slide-canvas"
          style={{
            aspectRatio: pageAspectRatio,
            width: zoom === "fit" ? undefined : `${zoom}%`,
          }}
        >
          <GoogleSlideVisual
            source={thumbnail}
            error={thumbnailError}
            slideNumber={selected + 1}
            text={slide?.text}
          />
          {editorOpen ? shapeFields.map((field) => (
            field.x != null && field.y != null && field.width != null && field.height != null ? (
              <textarea
                className="google-slide-inline-editor"
                aria-label={`Edit ${field.label.toLocaleLowerCase()}`}
                key={field.id}
                value={drafts[field.id] ?? ""}
                data-active={field.id === activeTextFieldId ? "true" : undefined}
                style={{
                  left: `${field.x * 100}%`,
                  top: `${field.y * 100}%`,
                  width: `${field.width * 100}%`,
                  height: `${field.height * 100}%`,
                  ...(field.id === activeTextFieldId && activeTextFormat ? {
                    color: activeTextFormat.color,
                    fontSize: `${activeTextFormat.fontSize}px`,
                    fontWeight: activeTextFormat.bold ? 700 : undefined,
                    fontStyle: activeTextFormat.italic ? "italic" : undefined,
                    textDecoration: activeTextFormat.underline ? "underline" : undefined,
                    textAlign: activeTextFormat.alignment === "JUSTIFIED"
                      ? "justify"
                      : activeTextFormat.alignment.toLocaleLowerCase() as CSSProperties["textAlign"],
                  } : {}),
                }}
                onFocus={(event) => {
                  setActiveTextFieldId(field.id);
                  setTextSelection({
                    start: event.currentTarget.selectionStart,
                    end: event.currentTarget.selectionEnd,
                  });
                  setSlideFormat(googleSlideTextFormat(field, event.currentTarget.selectionStart));
                  setSlideFormatError(null);
                }}
                onChange={(event) => setDrafts((current) => ({
                  ...current,
                  [field.id]: event.currentTarget.value,
                }))}
                onSelect={(event) => {
                  setTextSelection({
                    start: event.currentTarget.selectionStart,
                    end: event.currentTarget.selectionEnd,
                  });
                  setSlideFormat(null);
                }}
                onBlur={(event) => {
                  if (!(event.relatedTarget instanceof HTMLElement && event.relatedTarget.closest("[data-google-format-toolbar]")))
                    void slideAutosave.flush();
                }}
              />
            ) : null
          )) : null}
        </div>
        {thumbnailError ? <p className="sheet-hint error">{thumbnailError}</p> : null}
        {editorOpen && shapeFields.some((field) => field.x == null || field.y == null || field.width == null || field.height == null) ? (
          <small className="google-slide-layout-hint">Some unsupported rotated text boxes remain view-only.</small>
        ) : null}
        {notesOpen ? (
          <label className="google-slide-inline-notes">
            <strong>Speaker notes</strong>
            {canEdit && notesField ? (
              <textarea
                aria-label="Edit speaker notes"
                value={drafts[notesField.id] ?? ""}
                onChange={(event) => setDrafts((current) => ({
                  ...current,
                  [notesField.id]: event.currentTarget.value,
                }))}
                onBlur={() => void slideAutosave.flush()}
              />
            ) : <p>{slide?.notes?.trim() || "No speaker notes on this slide."}</p>}
          </label>
        ) : null}
        {slideAutosave.error || slideFormatError
          ? <p className="sheet-hint error" role="alert">{slideAutosave.error || slideFormatError}</p>
          : null}
      </div>
      </div>
      {presenting && typeof document !== "undefined" ? createPortal(
        <div
          className="google-slides-presentation"
          role="dialog"
          aria-modal="true"
          aria-label={`Presenting slide ${selected + 1} of ${slides.length}`}
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
                text={slide?.text}
              />
            </div>
          </div>
          <div className="google-slides-presentation-controls">
            <button type="button" disabled={selected === 0} onClick={() => selectRelative(-1)}>
              <ArrowLeft size={16} aria-hidden="true" /> Previous
            </button>
            <output aria-label={`Slide ${selected + 1} of ${slides.length}`} aria-live="polite">
              {selected + 1} / {slides.length}
            </output>
            <button type="button" disabled={selected === slides.length - 1} onClick={() => selectRelative(1)}>
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
  const runAt = (runs: GoogleSlideEditableField["styleRuns"]) =>
    runs?.find((run) => run.start <= index && index < run.end) ?? runs?.[runs.length - 1];
  const textStyle = runAt(field.styleRuns)?.style ?? {};
  const paragraphStyle = runAt(field.paragraphRuns)?.style ?? {};
  return {
    bold: textStyle.bold === true,
    italic: textStyle.italic === true,
    underline: textStyle.underline === true,
    fontSize: googleFontSize(textStyle.fontSize, 14),
    color: googleColorHex(textStyle.foregroundColor),
    alignment: googleTextAlignment(paragraphStyle.alignment),
  };
}

function GoogleSlideVisual({
  source,
  error,
  slideNumber,
  text,
}: {
  source: string | null;
  error: string | null;
  slideNumber: number;
  text?: string | null;
}) {
  return source ? <img src={source} alt={`Slide ${slideNumber}`} /> : (
    <div className="google-slide-fallback">
      {!error ? <span className="spinner" aria-hidden="true" /> : null}
      <strong>Slide {slideNumber}</strong>
      <p>{text || "No text on this slide."}</p>
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
    ? `${label} · Double-click text to edit`
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
