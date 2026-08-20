import { Fragment, useEffect, useMemo, useState, type ReactNode } from "react";
import { getCodexRateLimits, isCodexModel } from "../api";
import type {
  QuickSummary,
  QuickSummaryRow,
  QuickSummaryRowKind,
  QuickSummarySectionId,
  QuickSummarySource,
} from "../lib/quickSummary";
import { codexLimitsFromRateLimitPayload, formatProviderLimits } from "../lib/usageMetrics";
import {
  Archive,
  Bolt,
  Check,
  ChevronDown,
  Copy,
  Cube,
  FileText,
  Folder,
  Gear,
  GitBranch,
  Globe,
  Lightbulb,
  Paperclip,
  Shield,
  Sliders,
  Sparkles,
  Terminal,
  X,
} from "./icons";

const SOURCE_LIMIT = 5;

function toneClass(tone?: QuickSummaryRow["tone"]): string {
  return tone ? ` ${tone}` : "";
}

function rowValue(row: QuickSummaryRow): ReactNode {
  const diff = row.kind === "workspace" && row.value.match(/^(.*)(\+\S+) (-\S+)$/);
  if (!diff) return row.value;
  return <>{diff[1]}<span className="git-diff-stat-add">{diff[2]}</span>{" "}<span className="git-diff-stat-delete">{diff[3]}</span></>;
}

function rowIcon(row: QuickSummaryRow): ReactNode {
  switch (row.kind) {
    case "workspace":
      return <Folder size={13} />;
    case "browser":
      return <Globe size={13} />;
    case "model":
      return <Sparkles size={13} />;
    case "activity":
      return <Terminal size={13} />;
    case "goal":
      return <Lightbulb size={13} />;
    case "plan":
      return <Check size={13} />;
    case "privacy":
      return <Shield size={13} />;
    case "memory":
      return <Archive size={13} />;
    case "usage":
      return <Bolt size={13} />;
    case "limits":
      return <Sliders size={13} />;
    case "context":
      switch (row.label.toLowerCase()) {
        case "prompt estimate":
          return <Cube size={13} />;
        case "conversation":
          return <Copy size={13} />;
        case "repository rules":
          return <GitBranch size={13} />;
        case "skills":
          return <Bolt size={13} />;
        case "memory":
          return <Archive size={13} />;
        case "tool definitions":
          return <Gear size={13} />;
        case "context warnings":
          return <Shield size={13} />;
        default:
          return <FileText size={13} />;
      }
    default:
      return <FileText size={13} />;
  }
}

function sourceIcon(source: QuickSummarySource): ReactNode {
  if (source.kind === "attachment") return <Paperclip size={13} />;
  if (source.kind === "memory") return <Sparkles size={13} />;
  return <FileText size={13} />;
}

function ContextRow({ row, onClick, className = "" }: { row: QuickSummaryRow; onClick?: () => void; className?: string }) {
  const classes = `quick-summary-row${className ? ` ${className}` : ""}${toneClass(row.tone)}`;
  const content = (
    <>
      <span className="quick-summary-row-icon" aria-hidden="true">{rowIcon(row)}</span>
      <span className="quick-summary-row-copy">
        <strong>{row.label}</strong>
        <small>{rowValue(row)}</small>
        {row.meta && <em>{row.meta}</em>}
      </span>
    </>
  );
  if (!onClick) {
    return <div className={classes} title={row.title}>{content}</div>;
  }
  return (
    <button className={classes} type="button" title={row.title} onClick={onClick}>
      {content}
    </button>
  );
}

function PromptContext({ summary, row, rows }: { summary: QuickSummary; row: QuickSummaryRow; rows: QuickSummaryRow[] }) {
  const sources = summary.context?.sources ?? [];
  const ruleRows = sources.map((source) => (
    <ContextRow
      key={`rule:${source.path}`}
      className="quick-summary-rule-source"
      row={{
        kind: "context",
        label: source.path.split(/[\\/]/).pop() || source.path,
        value: source.status === "loaded" ? `${source.tokens.toLocaleString()} tokens` : source.status,
        meta: source.family,
        title: source.path,
        tone: source.status === "loaded" ? undefined : "warning",
      }}
    />
  ));
  const hasRepositoryRules = rows.some((detail) => detail.label.toLowerCase() === "repository rules");

  return (
    <details className="quick-summary-prompt">
      <summary className={`quick-summary-row${toneClass(row.tone)}`} title={row.title}>
        <span className="quick-summary-row-icon" aria-hidden="true">{rowIcon(row)}</span>
        <span className="quick-summary-row-copy">
          <strong>Prompt context</strong>
          <small>{rowValue(row)}</small>
          {row.meta && <em>{row.meta}</em>}
        </span>
        <ChevronDown className="quick-summary-prompt-chevron" size={12} aria-hidden="true" />
      </summary>
      <div className="quick-summary-prompt-details">
        {rows.map((detail) => (
          <Fragment key={`${detail.kind}:${detail.label}`}>
            <ContextRow row={detail} />
            {detail.label.toLowerCase() === "repository rules" && ruleRows}
          </Fragment>
        ))}
        {!hasRepositoryRules && ruleRows}
      </div>
    </details>
  );
}

function SourceRow({ source, onClick }: { source: QuickSummarySource; onClick: () => void }) {
  return (
    <button
      className="quick-summary-row quick-summary-source-row"
      type="button"
      title={`Open ${source.label}`}
      aria-label={`Open ${source.kind} ${source.label}`}
      onClick={onClick}
    >
      <span className="quick-summary-row-icon" aria-hidden="true">{sourceIcon(source)}</span>
      <span className="quick-summary-row-copy">
        <strong>{source.label.split(/[\\/]/).filter(Boolean).pop() || source.label}</strong>
      </span>
    </button>
  );
}

function SectionHeader({
  id,
  label,
  collapsed,
  onCollapsedChange,
}: {
  id: QuickSummarySectionId;
  label: string;
  collapsed: boolean;
  onCollapsedChange: (id: QuickSummarySectionId, collapsed: boolean) => void;
}) {
  return (
    <h3>
      <button
        className="quick-summary-section-toggle"
        type="button"
        data-testid={`quick-summary-section-${id}`}
        aria-expanded={!collapsed}
        aria-controls={`quick-summary-${id}-content`}
        onClick={() => onCollapsedChange(id, !collapsed)}
      >
        <span>{label}</span>
        <ChevronDown size={12} aria-hidden="true" />
      </button>
    </h3>
  );
}

export function QuickSummaryPanel({
  summary,
  open,
  workerPanel,
  collapsedSections,
  canOpenGit,
  onOpenChange,
  onSectionCollapsedChange,
  onOpenGit,
  onOpenGoal,
  onOpenSource,
}: {
  summary: QuickSummary;
  open: boolean;
  workerPanel: ReactNode;
  collapsedSections: QuickSummarySectionId[];
  canOpenGit: boolean;
  onOpenChange: (open: boolean) => void;
  onSectionCollapsedChange: (id: QuickSummarySectionId, collapsed: boolean) => void;
  onOpenGit: () => void;
  onOpenGoal: () => void;
  onOpenSource: (source: QuickSummarySource) => void;
}) {
  const rows = Array.isArray(summary?.rows) ? summary.rows : [];
  const sources = Array.isArray(summary?.sources) ? summary.sources : [];
  const [liveQuota, setLiveQuota] = useState("");
  const [sourcesExpanded, setSourcesExpanded] = useState(false);
  const model = summary?.model?.trim() ?? "";

  useEffect(() => {
    if (!open || !isCodexModel(model)) {
      setLiveQuota("");
      return;
    }
    let cancelled = false;
    const refresh = async () => {
      if (document.visibilityState === "hidden") return;
      try {
        const value = formatProviderLimits(codexLimitsFromRateLimitPayload(await getCodexRateLimits()));
        if (!cancelled) setLiveQuota(value ?? "");
      } catch {
        if (!cancelled) setLiveQuota("");
      }
    };
    void refresh();
    const timer = setInterval(() => void refresh(), 60_000);
    const onVisible = () => void refresh();
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      cancelled = true;
      clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [model, open]);

  const displayRows = useMemo(() => {
    if (!liveQuota) return rows;
    const quota: QuickSummaryRow = { kind: "limits", label: "Provider quota", value: liveQuota };
    const index = rows.findIndex((row) => row.kind === "limits");
    if (index >= 0) return rows.map((row, rowIndex) => rowIndex === index ? quota : row);
    const modelIndex = rows.findIndex((row) => row.kind === "model");
    return [...rows.slice(0, modelIndex + 1), quota, ...rows.slice(modelIndex + 1)];
  }, [liveQuota, rows]);

  const groups: Array<{ id: QuickSummarySectionId; label: string; kinds: QuickSummaryRowKind[] }> = [
    { id: "environment", label: "Environment", kinds: ["workspace", "browser"] },
    { id: "task", label: "Task", kinds: ["goal", "plan"] },
    { id: "activity", label: "Activity", kinds: ["activity"] },
    { id: "context", label: "Context", kinds: ["model", "context", "usage", "limits", "privacy", "memory"] },
  ];
  const sourcesCollapsed = collapsedSections.includes("sources");

  return (
    <aside
      id="quick-summary-panel"
      className={`quick-summary-panel${open ? " open" : ""}`}
      data-testid="quick-summary-panel"
      aria-labelledby={open ? "quick-summary-title" : undefined}
      aria-hidden={!open}
    >
      {open && (
        <div className="quick-summary-card">
          <div className="quick-summary-toolbar">
            <strong id="quick-summary-title">Context</strong>
            <button className="icon-btn quick-summary-close" type="button" title="Close context" aria-label="Close context" onClick={() => onOpenChange(false)}>
              <X size={14} />
            </button>
          </div>
          <div className="quick-summary-scroll">
            {workerPanel}
            {groups.map((group) => {
              const sectionRows = displayRows.filter((row) => group.kinds.includes(row.kind));
              if (!sectionRows.length) return null;
              const collapsed = collapsedSections.includes(group.id);
              const promptRow = group.id === "context"
                ? sectionRows.find((row) => row.kind === "context" && row.label.toLowerCase() === "prompt estimate")
                : undefined;
              const promptDetails = promptRow
                ? sectionRows.filter((row) => row.kind === "context" && row !== promptRow)
                : [];
              return (
                <section className="quick-summary-section" key={group.id} aria-label={group.label}>
                  <SectionHeader id={group.id} label={group.label} collapsed={collapsed} onCollapsedChange={onSectionCollapsedChange} />
                  <div
                    className="context-section-reveal"
                    id={`quick-summary-${group.id}-content`}
                    data-collapsed={collapsed || undefined}
                    aria-hidden={collapsed}
                  >
                    <div className="context-section-inner quick-summary-section-content">
                      {sectionRows.map((row) => {
                        if (row === promptRow) {
                          return <PromptContext key="prompt-context" summary={summary} row={row} rows={promptDetails} />;
                        }
                        if (promptRow && row.kind === "context") return null;
                        return (
                          <ContextRow
                            key={`${row.kind}:${row.label}`}
                            row={row}
                            onClick={
                              row.kind === "workspace" && canOpenGit
                                ? onOpenGit
                                : row.kind === "goal"
                                  ? onOpenGoal
                                  : undefined
                            }
                          />
                        );
                      })}
                      {group.id === "context" && !promptRow && summary.context?.sources.map((source) => (
                        <ContextRow
                          key={`rule:${source.path}`}
                          row={{
                            kind: "context",
                            label: source.path.split(/[\\/]/).pop() || source.path,
                            value: source.status === "loaded" ? `${source.tokens.toLocaleString()} tokens` : source.status,
                            meta: source.family,
                            title: source.path,
                            tone: source.status === "loaded" ? undefined : "warning",
                          }}
                        />
                      ))}
                    </div>
                  </div>
                </section>
              );
            })}
            <section className="quick-summary-section" aria-label="Sources">
              <SectionHeader
                id="sources"
                label="Sources"
                collapsed={sourcesCollapsed}
                onCollapsedChange={onSectionCollapsedChange}
              />
              <div
                className="context-section-reveal"
                id="quick-summary-sources-content"
                data-collapsed={sourcesCollapsed || undefined}
                aria-hidden={sourcesCollapsed}
              >
                <div className="context-section-inner quick-summary-section-content">
                  {sources.length ? (
                    <>
                      {sources.slice(0, sourcesExpanded ? sources.length : SOURCE_LIMIT).map((source) => (
                        <SourceRow
                          key={`${source.kind}:${source.label}`}
                          source={source}
                          onClick={() => onOpenSource(source)}
                        />
                      ))}
                      {sources.length > SOURCE_LIMIT && (
                        <button
                          className="quick-summary-more"
                          type="button"
                          aria-expanded={sourcesExpanded}
                          aria-label={sourcesExpanded ? "Show fewer sources" : `Show ${sources.length - SOURCE_LIMIT} more sources`}
                          onClick={() => setSourcesExpanded((expanded) => !expanded)}
                        >
                          {sourcesExpanded ? "Show less" : `${sources.length - SOURCE_LIMIT} more`}
                        </button>
                      )}
                    </>
                  ) : (
                    <div className="quick-summary-empty">None</div>
                  )}
                </div>
              </div>
            </section>
          </div>
        </div>
      )}
    </aside>
  );
}
