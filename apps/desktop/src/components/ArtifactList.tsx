import { useEffect, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import type { ArtifactFileStatus, ArtifactOpenTarget, ArtifactWritePreview, ChatArtifact, SavedArtifactFile } from "../api";
import type { ArtifactRevision, ArtifactRevisionChoice } from "../lib/artifactRevisions";
import { artifactDisposition, defaultArtifactTargetPath, formatBytes, isFileArtifact, isPreviewableArtifact } from "../lib/artifacts";
import { useContextMenu } from "./ContextMenu";
import { Check, Code, Copy, Download, Eye, FileText, Folder, Search } from "./icons";

type SaveArtifactOptions = {
  overwrite?: boolean;
  path?: string;
  source?: SavedArtifactFile["source"];
};

type BatchApplyResult = {
  artifactId: string;
  path: string;
  status: "success" | "error" | "unchanged";
  message: string;
};

type ArtifactCardItem = {
  cardId: string;
  artifact: ChatArtifact;
  revision?: ArtifactRevision;
  revisionChoice?: ArtifactRevisionChoice;
};

const MAX_VISIBLE_DIFF_LINES = 80;

export function ArtifactList({
  artifacts,
  currentSessionId,
  onOpenPreview,
  onSaveToWorkspace,
  onPreviewArtifact,
  onCheckSavedArtifact,
  onOpenSavedArtifact,
  revisionForArtifact,
  hiddenArtifactIds,
  autoSaveArtifacts = false,
  storageLabel = "folder",
}: {
  artifacts?: ChatArtifact[];
  currentSessionId?: string;
  onOpenPreview?: (artifact: ChatArtifact, revision?: ArtifactRevision) => void;
  onSaveToWorkspace?: (artifact: ChatArtifact, options?: SaveArtifactOptions, revision?: ArtifactRevision) => Promise<SavedArtifactFile>;
  onPreviewArtifact?: (artifact: ChatArtifact, path?: string, revision?: ArtifactRevision) => Promise<ArtifactWritePreview>;
  onCheckSavedArtifact?: (saved: SavedArtifactFile) => Promise<ArtifactFileStatus>;
  onOpenSavedArtifact?: (saved: SavedArtifactFile, target: ArtifactOpenTarget) => Promise<void>;
  revisionForArtifact?: (artifactIndex: number) => ArtifactRevisionChoice | undefined;
  hiddenArtifactIds?: ReadonlySet<string>;
  autoSaveArtifacts?: boolean;
  storageLabel?: string;
}) {
  const { openContextMenu } = useContextMenu();
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [savedById, setSavedById] = useState<Record<string, SavedArtifactFile>>({});
  const [statusById, setStatusById] = useState<Record<string, ArtifactFileStatus>>({});
  const [previewById, setPreviewById] = useState<Record<string, ArtifactWritePreview>>({});
  const [conflictById, setConflictById] = useState<Record<string, string>>({});
  const [errorById, setErrorById] = useState<Record<string, string>>({});
  const [targetPathById, setTargetPathById] = useState<Record<string, string>>({});
  const [batchSelectionById, setBatchSelectionById] = useState<Record<string, boolean>>({});
  const [diffExpandedById, setDiffExpandedById] = useState<Record<string, boolean>>({});
  const [reviewedAtById, setReviewedAtById] = useState<Record<string, number>>({});
  const [savingId, setSavingId] = useState<string | null>(null);
  const [previewingId, setPreviewingId] = useState<string | null>(null);
  const [autoSavingById, setAutoSavingById] = useState<Record<string, boolean>>({});
  const [selectedRevisionByKey, setSelectedRevisionByKey] = useState<Record<string, number>>({});
  const [batchBusy, setBatchBusy] = useState<"review" | "apply" | null>(null);
  const [batchStatus, setBatchStatus] = useState<string | null>(null);
  const [batchResults, setBatchResults] = useState<BatchApplyResult[]>([]);
  const autoSaveStartedRef = useRef(new Set<string>());

  function selectedRevision(choice: ArtifactRevisionChoice | undefined): ArtifactRevision | undefined {
    if (!choice) return undefined;
    const selectedNumber = selectedRevisionByKey[choice.group.key] ?? choice.revision.revisionNumber;
    return choice.group.revisions.find((revision) => revision.revisionNumber === selectedNumber) ?? choice.revision;
  }

  function cardItemFor(base: ChatArtifact, artifactIndex: number): ArtifactCardItem {
    const revisionChoice = revisionForArtifact?.(artifactIndex);
    const revision = selectedRevision(revisionChoice);
    return {
      cardId: base.id,
      artifact: revision?.artifact ?? base,
      revision,
      revisionChoice,
    };
  }

  function setSelectedRevision(choice: ArtifactRevisionChoice, revisionNumber: number) {
    setSelectedRevisionByKey((items) => ({ ...items, [choice.group.key]: revisionNumber }));
    clearBatchProgress();
  }

  const candidateItems = artifacts?.flatMap((base, artifactIndex): ArtifactCardItem[] => {
    const item = cardItemFor(base, artifactIndex);
    const saved = savedById[item.artifact.id] ?? item.artifact.saved;
    return !hiddenArtifactIds?.has(base.id) && saved?.source !== "tool_write" && saved?.source !== "auto_artifact" ? [item] : [];
  }) ?? [];

  useEffect(() => {
    if (!candidateItems.length || !onCheckSavedArtifact) return;
    let cancelled = false;
    for (const { artifact } of candidateItems) {
      if (!isFileArtifact(artifact)) continue;
      const saved = savedById[artifact.id] ?? artifact.saved;
      if (!saved) continue;
      const current = statusById[artifact.id];
      if (current?.path === saved.path) continue;
      void onCheckSavedArtifact(saved)
        .then((status) => {
          if (cancelled) return;
          setStatusById((items) => ({ ...items, [artifact.id]: status }));
        })
        .catch(() => {
          if (cancelled) return;
          setStatusById((items) => ({
            ...items,
            [artifact.id]: { path: saved.path, exists: false, is_file: false, is_dir: false, bytes: null },
          }));
        });
    }
    return () => {
      cancelled = true;
    };
  }, [candidateItems, onCheckSavedArtifact, savedById, statusById]);

  useEffect(() => {
    if (!autoSaveArtifacts || !onPreviewArtifact || !onSaveToWorkspace || !candidateItems.length) return;
    for (const item of candidateItems) {
      const { artifact, revision } = item;
      if (!isFileArtifact(artifact)) continue;
      if (savedById[artifact.id] || artifact.saved || previewById[artifact.id] || errorById[artifact.id] || autoSavingById[artifact.id]) continue;
      const path = targetPathFor(artifact);
      if (!path) continue;
      const key = `${artifact.id}\0${path}`;
      if (autoSaveStartedRef.current.has(key)) continue;
      autoSaveStartedRef.current.add(key);
      setAutoSavingById((items) => ({ ...items, [artifact.id]: true }));
      void onPreviewArtifact(artifact, path, revision)
        .then(async (preview) => {
          const saved = await onSaveToWorkspace(artifact, { path, overwrite: preview.exists, source: "auto_artifact" }, revision);
          rememberSaved(artifact.id, saved);
        })
        .catch((e) => {
          rememberArtifactError(artifact.id, errorMessage(e));
        })
        .finally(() => {
          setAutoSavingById((items) => {
            const next = { ...items };
            delete next[artifact.id];
            return next;
          });
        });
    }
  }, [autoSaveArtifacts, candidateItems, onPreviewArtifact, onSaveToWorkspace, savedById, previewById, errorById, autoSavingById, targetPathById]);

  const visibleItems = candidateItems.filter((item) => {
    if (!autoSaveArtifacts || !onPreviewArtifact || !onSaveToWorkspace) return true;
    if (!isFileArtifact(item.artifact)) return true;
    if (!targetPathFor(item.artifact)) return true;
    return Boolean(savedById[item.artifact.id] || item.artifact.saved || previewById[item.artifact.id] || errorById[item.artifact.id]);
  });
  const fileItems = visibleItems.filter((item) => isFileArtifact(item.artifact));

  if (!visibleItems.length) return null;

  async function copyArtifact(artifact: ChatArtifact) {
    await navigator.clipboard?.writeText(artifact.content);
    setCopiedId(artifact.id);
    window.setTimeout(() => setCopiedId((id) => (id === artifact.id ? null : id)), 1200);
  }

  function defaultTargetPath(artifact: ChatArtifact): string {
    return defaultArtifactTargetPath(artifact);
  }

  function targetPathFor(artifact: ChatArtifact): string {
    const raw = targetPathById[artifact.id] ?? defaultTargetPath(artifact);
    return raw.trim();
  }

  function updateTargetPath(artifact: ChatArtifact, path: string) {
    setTargetPathById((items) => ({ ...items, [artifact.id]: path }));
    setPreviewById((items) => {
      const next = { ...items };
      delete next[artifact.id];
      return next;
    });
    setDiffExpandedById((items) => {
      const next = { ...items };
      delete next[artifact.id];
      return next;
    });
    setReviewedAtById((items) => {
      const next = { ...items };
      delete next[artifact.id];
      return next;
    });
    setConflictById((items) => {
      const next = { ...items };
      delete next[artifact.id];
      return next;
    });
    clearBatchProgress();
    clearArtifactError(artifact.id);
  }

  function isArtifactSelected(item: ArtifactCardItem): boolean {
    return batchSelectionById[item.cardId] ?? true;
  }

  function selectedItemsForBatch(): ArtifactCardItem[] {
    return fileItems.filter((item) => isArtifactSelected(item));
  }

  function clearBatchProgress() {
    setBatchStatus(null);
    setBatchResults([]);
  }

  function toggleArtifactSelected(item: ArtifactCardItem) {
    setBatchSelectionById((items) => ({ ...items, [item.cardId]: !(items[item.cardId] ?? true) }));
    clearBatchProgress();
  }

  function setAllArtifactsSelected(selected: boolean) {
    if (!fileItems.length) return;
    setBatchSelectionById((items) => {
      const next = { ...items };
      for (const item of fileItems) next[item.cardId] = selected;
      return next;
    });
    clearBatchProgress();
  }

  function errorMessage(e: unknown): string {
    return e instanceof Error ? e.message : String(e);
  }

  function rememberArtifactError(artifactId: string, message: string) {
    setErrorById((items) => ({ ...items, [artifactId]: message }));
  }

  function clearArtifactError(artifactId: string) {
    setErrorById((items) => {
      const next = { ...items };
      delete next[artifactId];
      return next;
    });
  }

  async function saveToWorkspace(item: ArtifactCardItem, overwrite = false) {
    if (!onSaveToWorkspace) return;
    const { artifact, revision } = item;
    const path = targetPathFor(artifact);
    if (!path) {
      rememberArtifactError(artifact.id, "Enter a target path before saving this artifact.");
      return;
    }
    setSavingId(artifact.id);
    clearArtifactError(artifact.id);
    try {
      const saved = await onSaveToWorkspace(artifact, { overwrite, path }, revision);
      rememberSaved(artifact.id, saved);
    } catch (e) {
      const message = errorMessage(e);
      if (message.toLowerCase().includes("already exists")) {
        setConflictById((items) => ({ ...items, [artifact.id]: message }));
      } else {
        rememberArtifactError(artifact.id, message);
      }
    } finally {
      setSavingId((id) => (id === artifact.id ? null : id));
    }
  }

  function rememberSaved(artifactId: string, saved: SavedArtifactFile) {
    setSavedById((items) => ({ ...items, [artifactId]: saved }));
    setStatusById((items) => ({
      ...items,
      [artifactId]: { path: saved.path, exists: true, is_file: true, is_dir: false, bytes: saved.bytes },
    }));
    setPreviewById((items) => {
      const next = { ...items };
      delete next[artifactId];
      return next;
    });
    setConflictById((items) => {
      const next = { ...items };
      delete next[artifactId];
      return next;
    });
    clearArtifactError(artifactId);
  }

  async function loadPreview(item: ArtifactCardItem): Promise<ArtifactWritePreview | null> {
    if (!onPreviewArtifact) return null;
    const { artifact, revision } = item;
    const path = targetPathFor(artifact);
    if (!path) {
      const message = "Enter a target path before reviewing this artifact.";
      rememberArtifactError(artifact.id, message);
      throw new Error(message);
    }
    clearArtifactError(artifact.id);
    try {
      const preview = await onPreviewArtifact(artifact, path, revision);
      setPreviewById((items) => ({ ...items, [artifact.id]: preview }));
      setDiffExpandedById((items) => ({ ...items, [artifact.id]: false }));
      setReviewedAtById((items) => ({ ...items, [artifact.id]: Date.now() }));
      return preview;
    } catch (e) {
      rememberArtifactError(artifact.id, errorMessage(e));
      throw e;
    }
  }

  async function previewChanges(item: ArtifactCardItem) {
    const { artifact } = item;
    setPreviewingId(artifact.id);
    try {
      await loadPreview(item);
    } catch {
      /* loadPreview stores the card-level error. */
    } finally {
      setPreviewingId((id) => (id === artifact.id ? null : id));
    }
  }

  async function previewAllArtifacts() {
    if (!fileItems.length || !onPreviewArtifact) return;
    const selectedItems = selectedItemsForBatch().filter((item) => targetPathFor(item.artifact));
    if (selectedItems.length === 0) {
      setBatchStatus("Enter a target path for at least one selected artifact.");
      return;
    }
    setBatchBusy("review");
    setBatchStatus(null);
    setBatchResults([]);
    try {
      const previews = await Promise.all(selectedItems.map((item) => loadPreview(item)));
      const changed = previews.filter((preview) => preview?.changed).length;
      const unchanged = previews.length - changed;
      setBatchStatus(reviewStatus(selectedItems.length, changed, unchanged));
    } catch (e) {
      setBatchStatus(`Preview failed: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBatchBusy(null);
    }
  }

  async function applyAllArtifacts() {
    if (!fileItems.length || !onSaveToWorkspace) return;
    const selectedItems = selectedItemsForBatch().filter((item) => targetPathFor(item.artifact));
    if (selectedItems.length === 0) {
      setBatchStatus("Enter a target path for at least one selected artifact.");
      return;
    }
    setBatchBusy("apply");
    setBatchStatus(null);
    setBatchResults([]);
    try {
      const results: BatchApplyResult[] = [];
      for (const item of selectedItems) {
        const { artifact, revision } = item;
        const path = targetPathFor(artifact);
        try {
          const preview = previewById[artifact.id] ?? (onPreviewArtifact ? await loadPreview(item) : null);
          if (preview?.changed === false) {
            results.push({ artifactId: artifact.id, path, status: "unchanged", message: "Already up to date" });
            continue;
          }
          const saved = await onSaveToWorkspace(artifact, { overwrite: Boolean(preview?.exists), path }, revision);
          rememberSaved(artifact.id, saved);
          results.push({ artifactId: artifact.id, path, status: "success", message: "Applied" });
        } catch (e) {
          const message = errorMessage(e);
          rememberArtifactError(artifact.id, message);
          results.push({ artifactId: artifact.id, path, status: "error", message });
        }
      }
      const applied = results.filter((result) => result.status === "success").length;
      const failed = results.filter((result) => result.status === "error").length;
      const unchanged = results.filter((result) => result.status === "unchanged").length;
      setBatchResults(results);
      setBatchStatus(applyStatus(applied, unchanged, failed));
    } catch (e) {
      setBatchStatus(`Apply failed: ${errorMessage(e)}`);
    } finally {
      setBatchBusy(null);
    }
  }

  async function openSaved(artifactId: string, saved: SavedArtifactFile, target: ArtifactOpenTarget) {
    if (!onOpenSavedArtifact) return;
    clearArtifactError(artifactId);
    try {
      await onOpenSavedArtifact(saved, target);
    } catch (e) {
      setStatusById((items) => ({
        ...items,
        [artifactId]: { path: saved.path, exists: false, is_file: false, is_dir: false, bytes: null },
      }));
      rememberArtifactError(artifactId, `Could not open artifact: ${errorMessage(e)}`);
    }
  }

  function downloadArtifact(artifact: ChatArtifact) {
    const blob = new Blob([artifact.content], { type: artifact.mime || "text/plain" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = downloadName(artifact);
    link.click();
    URL.revokeObjectURL(url);
  }

  function toggleDiffExpanded(artifactId: string) {
    setDiffExpandedById((items) => ({ ...items, [artifactId]: !items[artifactId] }));
  }

  function batchResultSaved(result: BatchApplyResult): SavedArtifactFile {
    const artifact = visibleItems.find((item) => item.artifact.id === result.artifactId)?.artifact;
    return savedById[result.artifactId] ?? artifact?.saved ?? { path: result.path, bytes: 0, overwritten: false };
  }

  const batchSelectable = fileItems.length > 1 && Boolean(onSaveToWorkspace && onPreviewArtifact);
  const selectedArtifactCount = selectedItemsForBatch().length;

  return (
    <div className="artifact-list" data-testid="artifact-list">
      {batchSelectable && (
        <div className="artifact-batch">
          <div className="artifact-batch-top">
            <div className="artifact-batch-copy">
              <strong>{fileItems.length} generated files</strong>
              <span className="artifact-batch-selection" data-testid="artifact-batch-selection-count">
                {selectedArtifactCount} of {fileItems.length} selected
              </span>
              <span data-testid="artifact-batch-status">{batchStatus ?? "Review and apply this response as one change set."}</span>
            </div>
            <div className="artifact-batch-actions">
              <button
                className="artifact-overwrite"
                data-testid="artifact-batch-select-all"
                disabled={batchBusy != null || selectedArtifactCount === fileItems.length}
                onClick={() => setAllArtifactsSelected(true)}
              >
                All
              </button>
              <button
                className="artifact-overwrite"
                data-testid="artifact-batch-select-none"
                disabled={batchBusy != null || selectedArtifactCount === 0}
                onClick={() => setAllArtifactsSelected(false)}
              >
                None
              </button>
              <button
                className="artifact-overwrite"
                data-testid="artifact-batch-review"
                disabled={batchBusy != null || selectedArtifactCount === 0}
                onClick={() => void previewAllArtifacts()}
              >
                {batchBusy === "review" ? "Reviewing..." : "Review selected"}
              </button>
              <button
                className="artifact-overwrite"
                data-testid="artifact-batch-apply"
                disabled={batchBusy != null || selectedArtifactCount === 0}
                onClick={() => void applyAllArtifacts()}
              >
                {batchBusy === "apply" ? "Applying..." : "Apply selected"}
              </button>
            </div>
          </div>
          {batchResults.length > 0 && (
            <div className="artifact-batch-results" data-testid="artifact-batch-results">
              {batchResults.map((result) => {
                const saved = result.status === "error" ? null : batchResultSaved(result);
                return (
                  <div className={`artifact-batch-result ${result.status}`} data-testid="artifact-batch-result" key={result.artifactId}>
                    <span className="artifact-batch-result-name" title={result.path}>
                      {result.path}
                    </span>
                    <span className="artifact-batch-result-status">{batchResultLabel(result.status)}</span>
                    {saved && onOpenSavedArtifact && (
                      <span className="artifact-batch-result-actions">
                        <button
                          className="artifact-batch-result-action"
                          data-testid="artifact-batch-open-file"
                          title="Open file"
                          aria-label="Open batch result file"
                          onClick={() => void openSaved(result.artifactId, saved, "file")}
                        >
                          <Eye size={12} />
                        </button>
                        <button
                          className="artifact-batch-result-action"
                          data-testid="artifact-batch-open-folder"
                          title={`Show in ${storageLabel}`}
                          aria-label={`Show batch result in ${storageLabel}`}
                          onClick={() => void openSaved(result.artifactId, saved, "folder")}
                        >
                          <Folder size={12} />
                        </button>
                      </span>
                    )}
                    {result.status === "error" && <span className="artifact-batch-result-message">{result.message}</span>}
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}
      {visibleItems.map((item) => {
        const { artifact, revision, revisionChoice } = item;
        const revisionGroup = revisionChoice?.group;
        const fileArtifact = isFileArtifact(artifact);
        const disposition = artifactDisposition(artifact);
        const batchSelected = isArtifactSelected(item);
        const savedForMenu = savedById[artifact.id] ?? artifact.saved;
        const statusForMenu = savedForMenu ? statusById[artifact.id] : undefined;
        const savedFileAvailableForMenu = Boolean(savedForMenu && (!onCheckSavedArtifact || (statusForMenu?.exists && statusForMenu.is_file)));
        const missingTargetForMenu = fileArtifact && !targetPathFor(artifact);
        const conflictForMenu = conflictById[artifact.id];
        const openArtifactContextMenu = (event: ReactMouseEvent) => {
          openContextMenu(event, [
            ...(fileArtifact && onOpenPreview && isPreviewableArtifact(artifact) ? [{
              id: "open-preview",
              label: "Open preview",
              icon: <Eye size={13} />,
              action: () => onOpenPreview(artifact, revision),
            }] : []),
            ...(fileArtifact && onPreviewArtifact ? [{
              id: "review",
              label: "Review changes",
              icon: <Search size={13} />,
              disabled: missingTargetForMenu || previewingId === artifact.id,
              action: () => void previewChanges(item),
            }] : []),
            ...(fileArtifact && onSaveToWorkspace ? [{
              id: "save",
              label: savedForMenu ? `Save again to ${storageLabel}` : `Save to ${storageLabel}`,
              icon: <Folder size={13} />,
              disabled: missingTargetForMenu || savingId === artifact.id,
              action: () => void saveToWorkspace(item),
            }] : []),
            ...(conflictForMenu ? [{
              id: "overwrite",
              label: "Overwrite target",
              icon: <Folder size={13} />,
              danger: true,
              action: () => void saveToWorkspace(item, true),
            }] : []),
            {
              id: "copy",
              label: "Copy artifact",
              icon: <Copy size={13} />,
              separatorBefore: true,
              action: () => void copyArtifact(artifact),
            },
            {
              id: "download",
              label: "Download artifact",
              icon: <Download size={13} />,
              action: () => downloadArtifact(artifact),
            },
            ...(savedForMenu && savedFileAvailableForMenu && onOpenSavedArtifact ? [{
              id: "open-file",
              label: "Open saved file",
              icon: <Eye size={13} />,
              separatorBefore: true,
              action: () => void openSaved(artifact.id, savedForMenu, "file"),
            }, {
              id: "open-folder",
              label: `Show in ${storageLabel}`,
              icon: <Folder size={13} />,
              action: () => void openSaved(artifact.id, savedForMenu, "folder"),
            }] : []),
          ], artifact.filename ?? artifact.title);
        };
        return (
        <section className={`artifact-card${batchSelectable && fileArtifact && !batchSelected ? " deselected" : ""}`} data-testid="artifact-card" key={item.cardId} onContextMenu={openArtifactContextMenu}>
          {(() => {
            const saved = savedById[artifact.id] ?? artifact.saved;
            const status = saved ? statusById[artifact.id] : undefined;
            const checkingSavedFile = Boolean(saved && onCheckSavedArtifact && !status);
            const savedFileAvailable = Boolean(saved && (!onCheckSavedArtifact || (status?.exists && status.is_file)));
            const savedFileUnavailable = Boolean(saved && status && (!status.exists || !status.is_file));
            const conflict = conflictById[artifact.id];
            const error = errorById[artifact.id];
            const writePreview = previewById[artifact.id];
            const targetPath = targetPathFor(artifact);
            const missingTarget = !targetPath;
            const reviewedAt = reviewedAtById[artifact.id];
            const previewUnchanged = writePreview ? !writePreview.changed : false;
            const diffLines = writePreview ? writePreview.diff.split("\n") : [];
            const diffExpanded = Boolean(diffExpandedById[artifact.id]);
            const diffIsLarge = diffLines.length > MAX_VISIBLE_DIFF_LINES;
            const visibleDiffLines = diffIsLarge && !diffExpanded ? diffLines.slice(0, MAX_VISIBLE_DIFF_LINES) : diffLines;
            const hiddenDiffLineCount = Math.max(0, diffLines.length - visibleDiffLines.length);
            const summary = diffSummary(diffLines);
            return (
              <>
          <div className="artifact-icon" aria-hidden="true">
            {artifact.kind === "code" ? <Code size={16} /> : <FileText size={16} />}
          </div>
          <div className="artifact-body">
            <div className="artifact-head">
              {batchSelectable && fileArtifact && (
                <button
                  className={`artifact-select-toggle${batchSelected ? " selected" : ""}`}
                  data-testid="artifact-select-toggle"
                  type="button"
                  role="checkbox"
                  aria-checked={batchSelected}
                  title={batchSelected ? "Included in batch actions" : "Excluded from batch actions"}
                  aria-label={batchSelected ? "Exclude artifact from batch actions" : "Include artifact in batch actions"}
                  onClick={() => toggleArtifactSelected(item)}
                >
                  {batchSelected && <Check size={12} />}
                </button>
              )}
              <div className="artifact-title-group">
                <div className="artifact-title" title={artifact.filename ?? artifact.title}>
                  {artifact.filename ?? artifact.title}
                </div>
                <div className="artifact-meta">
                  {artifact.kind}
                  {artifact.language ? `/${artifact.language}` : ""} - {disposition} - {formatBytes(artifact.size)}
                </div>
                {revisionChoice && revisionGroup && revision && revisionGroup.revisions.length > 1 && (
                  <label className="artifact-revision-control" title={`${revisionGroup.label} revision`}>
                    <span>Version</span>
                    <select
                      data-testid="artifact-revision-select"
                      aria-label="Artifact revision"
                      value={revision.revisionNumber}
                      onChange={(event) => setSelectedRevision(revisionChoice, Number(event.currentTarget.value))}
                    >
                      {revisionGroup.revisions.map((itemRevision) => (
                        <option key={itemRevision.revisionNumber} value={itemRevision.revisionNumber}>
                          v{itemRevision.revisionNumber}{itemRevision.revisionNumber === itemRevision.totalRevisions ? " latest" : ""}
                        </option>
                      ))}
                    </select>
                  </label>
                )}
              </div>
              <div className="artifact-actions">
                {fileArtifact && onOpenPreview && isPreviewableArtifact(artifact) && (
                  <button
                    className="artifact-action"
                    data-testid="artifact-open-preview"
                    title="Open preview"
                    aria-label="Open preview"
                    onClick={() => onOpenPreview(artifact, revision)}
                  >
                    <Eye size={14} />
                  </button>
                )}
                {fileArtifact && onPreviewArtifact && (
                  <button
                    className="artifact-action"
                    data-testid="artifact-review-workspace"
                    title={missingTarget ? "Enter a target path" : previewingId === artifact.id ? "Reviewing changes" : "Review changes"}
                    aria-label={missingTarget ? "Enter a target path" : previewingId === artifact.id ? "Reviewing changes" : "Review changes"}
                    disabled={missingTarget || previewingId === artifact.id}
                    onClick={() => void previewChanges(item)}
                  >
                    <Search size={14} />
                  </button>
                )}
                {fileArtifact && onSaveToWorkspace && (
                  <button
                    className="artifact-action"
                    data-testid="artifact-save-workspace"
                    title={missingTarget ? "Enter a target path" : savedFileUnavailable ? `Resave to ${storageLabel}` : saved ? `Saved to ${storageLabel}` : `Save to ${storageLabel}`}
                    aria-label={missingTarget ? "Enter a target path" : savedFileUnavailable ? `Resave to ${storageLabel}` : saved ? `Saved to ${storageLabel}` : `Save to ${storageLabel}`}
                    disabled={missingTarget || savingId === artifact.id}
                    onClick={() => void saveToWorkspace(item)}
                  >
                    <Folder size={14} />
                  </button>
                )}
                <button
                  className="artifact-action"
                  data-testid="artifact-copy"
                  title={copiedId === artifact.id ? "Copied" : "Copy artifact"}
                  aria-label={copiedId === artifact.id ? "Copied" : "Copy artifact"}
                  onClick={() => void copyArtifact(artifact)}
                >
                  <Copy size={14} />
                </button>
                <button
                  className="artifact-action"
                  data-testid="artifact-download"
                  title="Download artifact"
                  aria-label="Download artifact"
                  onClick={() => downloadArtifact(artifact)}
                >
                  <Download size={14} />
                </button>
              </div>
            </div>
            {fileArtifact && (
              <label className="artifact-target">
                <span>Target</span>
                <input
                  className="artifact-target-input"
                  data-testid="artifact-target-path"
                  aria-label="Artifact target path"
                  value={targetPathById[artifact.id] ?? defaultTargetPath(artifact)}
                  placeholder="relative/path.ext"
                  onChange={(event) => updateTargetPath(artifact, event.currentTarget.value)}
                />
              </label>
            )}
            <pre className="artifact-preview">{previewText(artifact.content)}</pre>
            {fileArtifact && saved && (
              <div className="artifact-saved">
                <div className="artifact-saved-main">
                  <span className="artifact-saved-path" data-testid="artifact-saved-path" title={saved.path}>
                    {saved.path}
                  </span>
                  {checkingSavedFile && <span className="artifact-file-checking">Checking file...</span>}
                  {savedFileUnavailable && (
                    <span className="artifact-file-missing" data-testid="artifact-file-missing">
                      Saved file unavailable. Save again to recreate it.
                    </span>
                  )}
                  {savedFileAvailable && (
                    <>
                      <button
                        className="artifact-mini-action"
                        data-testid="artifact-open-file"
                        title="Open saved file"
                        aria-label="Open saved file"
                        onClick={() => void openSaved(artifact.id, saved, "file")}
                      >
                        <Eye size={13} />
                      </button>
                      <button
                        className="artifact-mini-action"
                        data-testid="artifact-open-folder"
                        title={`Show in ${storageLabel}`}
                        aria-label={`Show in ${storageLabel}`}
                        onClick={() => void openSaved(artifact.id, saved, "folder")}
                      >
                        <Folder size={13} />
                      </button>
                    </>
                  )}
                </div>
                <div className="artifact-saved-meta">
                  <span data-testid="artifact-saved-session">{savedSessionLabel(saved, currentSessionId)}</span>
                  {saved.savedAt && <span data-testid="artifact-saved-time">Saved {formatTraceTime(saved.savedAt)}</span>}
                </div>
              </div>
            )}
            {fileArtifact && conflict && (
              <div className="artifact-conflict" data-testid="artifact-conflict">
                <span>File already exists.</span>
                {onPreviewArtifact && (
                  <button
                    className="artifact-overwrite"
                    data-testid="artifact-preview-changes"
                    disabled={previewingId === artifact.id}
                    onClick={() => void previewChanges(item)}
                  >
                    Preview changes
                  </button>
                )}
                <button
                  className="artifact-overwrite"
                  data-testid="artifact-overwrite"
                  onClick={() => void saveToWorkspace(item, true)}
                >
                  Overwrite
                </button>
              </div>
            )}
            {error && (
              <div className="artifact-error" data-testid="artifact-error">
                {error}
              </div>
            )}
            {fileArtifact && writePreview && (
              <div className={`artifact-diff-preview${previewUnchanged ? " unchanged" : ""}`}>
                <div className="artifact-diff-head">
                  <span>
                    {previewUnchanged ? "No changes" : writePreview.exists ? "Changes against current file" : "New file preview"}
                    {!previewUnchanged && (
                      <span className="artifact-diff-summary" data-testid="artifact-diff-summary">
                        {summary.added} added, {summary.removed} removed
                      </span>
                    )}
                    {reviewedAt && (
                      <span className="artifact-reviewed-time" data-testid="artifact-reviewed-time">
                        Reviewed {formatTraceTime(reviewedAt)}
                      </span>
                    )}
                  </span>
                  {!previewUnchanged && (
                    <button
                      className="artifact-overwrite"
                      data-testid="artifact-preview-apply"
                      disabled={savingId === artifact.id}
                      onClick={() => void saveToWorkspace(item, true)}
                    >
                      Apply
                    </button>
                  )}
                </div>
                {previewUnchanged ? (
                  <div className="artifact-unchanged" data-testid="artifact-unchanged">
                    The target file already matches this artifact.
                  </div>
                ) : (
                  <pre className="artifact-diff" data-testid="artifact-preview-diff">
                    {visibleDiffLines.map(renderDiffLine)}
                  </pre>
                )}
                {!previewUnchanged && diffIsLarge && (
                  <button
                    className="artifact-diff-toggle"
                    data-testid="artifact-diff-toggle"
                    aria-expanded={diffExpanded}
                    onClick={() => toggleDiffExpanded(artifact.id)}
                  >
                    {diffExpanded ? `Show first ${MAX_VISIBLE_DIFF_LINES} lines` : `Show ${hiddenDiffLineCount} more lines`}
                  </button>
                )}
              </div>
            )}
          </div>
              </>
            );
          })()}
        </section>
        );
      })}
    </div>
  );
}

function downloadName(artifact: ChatArtifact): string {
  const raw = artifact.filename ?? artifact.title;
  return raw.split(/[\\/]/).pop() || "artifact.txt";
}

function artifactLabel(count: number): string {
  return count === 1 ? "artifact" : "artifacts";
}

function savedSessionLabel(saved: SavedArtifactFile, currentSessionId?: string): string {
  const sessionLabel = saved.sourceSessionId && currentSessionId && saved.sourceSessionId === currentSessionId
    ? "Saved in this app session"
    : "Saved in a previous app session";
  const turnLabel = typeof saved.sourceMessageIndex === "number" ? ` from turn ${saved.sourceMessageIndex + 1}` : "";
  const revisionLabel = typeof saved.sourceRevisionNumber === "number" ? `, revision ${saved.sourceRevisionNumber}` : "";
  return `${sessionLabel}${turnLabel}${revisionLabel}`;
}

function formatTraceTime(value: number): string {
  return new Date(value).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function reviewStatus(total: number, changed: number, unchanged: number): string {
  const prefix = `${total} ${artifactLabel(total)} reviewed; ${changed} changed`;
  return unchanged > 0 ? `${prefix}, ${unchanged} unchanged.` : `${prefix}.`;
}

function applyStatus(applied: number, unchanged: number, failed: number): string {
  if (failed > 0) {
    const unchangedPart = unchanged > 0 ? `; ${unchanged} unchanged` : "";
    return `${applied} applied${unchangedPart}; ${failed} failed.`;
  }
  if (unchanged > 0) return `${applied} applied; ${unchanged} unchanged.`;
  return `${applied} ${artifactLabel(applied)} applied.`;
}

function batchResultLabel(status: BatchApplyResult["status"]): string {
  if (status === "success") return "Applied";
  if (status === "unchanged") return "Unchanged";
  return "Failed";
}

function previewText(content: string): string {
  return content.split(/\r?\n/).slice(0, 8).join("\n");
}

function renderDiffLine(line: string, index: number) {
  const kind = diffLineKind(line);
  return (
    <span key={index} className={`artifact-diff-line ${kind}`} data-diff-kind={kind}>
      {line || " "}
    </span>
  );
}

function diffLineKind(line: string): "added" | "removed" | "hunk" | "file" | "context" {
  if (line.startsWith("@@")) return "hunk";
  if (line.startsWith("+++") || line.startsWith("---")) return "file";
  if (line.startsWith("+")) return "added";
  if (line.startsWith("-")) return "removed";
  return "context";
}

function diffSummary(lines: string[]): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const line of lines) {
    const kind = diffLineKind(line);
    if (kind === "added") added += 1;
    if (kind === "removed") removed += 1;
  }
  return { added, removed };
}
