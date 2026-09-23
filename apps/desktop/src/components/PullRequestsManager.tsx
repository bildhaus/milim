import {
  lazy,
  Suspense,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import {
  openExternalUrl,
  runWorkspaceGitAction,
  type PullRequestActor,
  type PullRequestDetails,
  type PullRequestListItem,
  type WorkspaceGitAction,
} from "../api";
import {
  readPullRequestCache,
  upsertPullRequestItems,
  writePullRequestCache,
  type PullRequestFilter,
  type PullRequestTab,
} from "../lib/pullRequestCache";
import {
  pullRequestActorAvatarUrls,
  pullRequestErrorPresentation,
  pullRequestReadiness,
} from "../lib/pullRequests";
import { sessionRecencyLabel } from "../lib/sessionRecency";
import { containerMax, usePaneResize } from "../ui/usePaneResize";
import {
  ArrowRight,
  Check,
  Code,
  ExternalLink,
  GitBranch,
  GitHub,
  GitPullRequest,
  Refresh,
  Search,
  X,
} from "./icons";
import { SheetDialog } from "./SheetDialog";
import { PaneResizeHandle } from "./PaneResizeHandle";

const Markdown = lazy(() =>
  import("./Markdown").then((module) => ({ default: module.Markdown })),
);

type ReviewAction = "approve" | "request_changes" | "comment";
type MergeMethod = "merge" | "squash" | "rebase";

function Actor({ actor }: { actor?: PullRequestActor }) {
  const avatarUrls = useMemo(
    () => pullRequestActorAvatarUrls(actor),
    [actor?.avatarUrl, actor?.login],
  );
  const [avatarIndex, setAvatarIndex] = useState(0);
  useEffect(() => setAvatarIndex(0), [avatarUrls]);
  const avatarUrl = avatarUrls[avatarIndex];
  return avatarUrl ? (
    <img
      src={avatarUrl}
      alt=""
      loading="lazy"
      referrerPolicy="no-referrer"
      onError={() => setAvatarIndex((index) => index + 1)}
    />
  ) : (
    <GitHub size={15} aria-hidden="true" />
  );
}

function relativeDate(value?: string): string {
  const timestamp = value ? new Date(value).getTime() : Number.NaN;
  return Number.isFinite(timestamp) ? sessionRecencyLabel(timestamp) : "";
}

const PULL_REQUEST_DETAIL_MIN_WIDTH = 320;

function titleCase(value?: string): string {
  const text = value?.toLowerCase().replace(/_/g, " ").trim() ?? "";
  return text ? text.charAt(0).toUpperCase() + text.slice(1) : "";
}

function parsePullRequestList(stdout: string): PullRequestListItem[] {
  const value: unknown = JSON.parse(stdout);
  if (!Array.isArray(value)) throw new Error("GitHub returned an invalid pull request list.");
  return value.filter(
    (item): item is PullRequestListItem =>
      Boolean(
        item &&
          typeof item === "object" &&
          typeof (item as PullRequestListItem).url === "string" &&
          typeof (item as PullRequestListItem).repository === "string" &&
          Number.isFinite((item as PullRequestListItem).number),
      ),
  );
}

export function PullRequestsManager({ onClose }: { onClose: () => void }) {
  const layoutRef = useRef<HTMLDivElement>(null);
  // The detail pane keeps at least 320px; CSS applies the same clamp live.
  const listResize = usePaneResize("pullRequestsList", {
    max: containerMax(layoutRef, PULL_REQUEST_DETAIL_MIN_WIDTH),
    targetRef: layoutRef,
    cssVar: "--pull-requests-list-width",
  });
  const [filter, setFilter] = useState<PullRequestFilter>("all");
  const [tab, setTab] = useState<PullRequestTab>("summary");
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<PullRequestListItem[]>([]);
  const [selectedUrl, setSelectedUrl] = useState("");
  const [detailsByUrl, setDetailsByUrl] = useState<
    Record<string, PullRequestDetails>
  >({});
  const [cacheHydrated, setCacheHydrated] = useState(false);
  const [listLoading, setListLoading] = useState(true);
  const [listRefreshing, setListRefreshing] = useState(false);
  const [detailsRefreshing, setDetailsRefreshing] = useState<Set<string>>(
    () => new Set(),
  );
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [comment, setComment] = useState("");
  const [reviewAction, setReviewAction] = useState<ReviewAction | null>(null);
  const [reviewBody, setReviewBody] = useState("");
  const [mergeMethod, setMergeMethod] = useState<MergeMethod | null>(null);
  const [actionBusy, setActionBusy] = useState<WorkspaceGitAction | null>(null);
  const detailsRequestsRef = useRef(new Set<string>());

  const selected = items.find((item) => item.url === selectedUrl) ?? null;
  const details = selected ? detailsByUrl[selected.url] ?? null : null;
  const detailsLoading = selected
    ? detailsRefreshing.has(selected.url)
    : false;
  const errorPresentation = useMemo(
    () => (error ? pullRequestErrorPresentation(error) : null),
    [error],
  );
  const visibleItems = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return items.filter((item) => {
      if (filter === "authored" && !item.authored) return false;
      if (filter === "reviewing" && !item.reviewing) return false;
      return (
        !needle ||
        item.title.toLowerCase().includes(needle) ||
        item.repository.toLowerCase().includes(needle) ||
        item.author?.login.toLowerCase().includes(needle)
      );
    });
  }, [filter, items, query]);

  useEffect(() => {
    let active = true;
    void readPullRequestCache().then((cache) => {
      if (!active) return;
      setItems(cache.items);
      setDetailsByUrl(cache.detailsByUrl);
      setSelectedUrl(
        cache.items.some((item) => item.url === cache.selectedUrl)
          ? cache.selectedUrl
          : cache.items[0]?.url ?? "",
      );
      setFilter(cache.filter);
      setQuery(cache.query);
      setTab(cache.tab);
      setListLoading(false);
      setCacheHydrated(true);
      void refreshList(cache.items.length === 0, cache.items.length > 0);
    }).catch((error) => {
      if (!active) return;
      setNotice(`Couldn't restore saved pull requests: ${error instanceof Error ? error.message : String(error)}`);
      setListLoading(false);
      setCacheHydrated(true);
      void refreshList(true, false);
    });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    if (selected) void refreshDetails(selected);
  }, [selected?.url]);

  useEffect(() => {
    if (!visibleItems.some((item) => item.url === selectedUrl)) {
      setSelectedUrl(visibleItems[0]?.url ?? "");
    }
  }, [selectedUrl, visibleItems]);

  useEffect(() => {
    if (!cacheHydrated) return;
    const timer = window.setTimeout(() => {
      void writePullRequestCache({
        items,
        detailsByUrl,
        selectedUrl,
        filter,
        query,
        tab,
      });
    }, 150);
    return () => window.clearTimeout(timer);
  }, [
    cacheHydrated,
    detailsByUrl,
    filter,
    items,
    query,
    selectedUrl,
    tab,
  ]);

  async function refreshList(
    showLoading = items.length === 0,
    hasVisibleItems = items.length > 0,
  ) {
    if (showLoading) setListLoading(true);
    setListRefreshing(true);
    if (!items.length) setError("");
    try {
      const result = await runWorkspaceGitAction("pr_list");
      if (!result.ok) throw new Error(result.message);
      const next = parsePullRequestList(result.stdout);
      setError("");
      setItems((current) =>
        upsertPullRequestItems(result.stderr.trim() ? current : [], next),
      );
      setSelectedUrl((current) =>
        current || next[0]?.url || "",
      );
      setNotice(result.stderr.trim() ? result.message : "");
    } catch (nextError) {
      const message =
        nextError instanceof Error
          ? nextError.message
          : "Couldn't load pull requests.";
      if (hasVisibleItems) setNotice(`Showing saved pull requests. ${message}`);
      else setError(message);
    } finally {
      setListLoading(false);
      setListRefreshing(false);
    }
  }

  async function refreshDetails(item = selected) {
    if (!item) return;
    if (detailsRequestsRef.current.has(item.url)) return;
    detailsRequestsRef.current.add(item.url);
    setDetailsRefreshing((current) => new Set(current).add(item.url));
    if (!detailsByUrl[item.url]) setError("");
    try {
      const result = await runWorkspaceGitAction("pr_view", {
        repository: item.repository,
        pull_request: item.number,
      });
      if (!result.ok || !result.pull_request) throw new Error(result.message);
      setError("");
      setDetailsByUrl((current) => ({
        ...current,
        [item.url]: result.pull_request!,
      }));
    } catch (nextError) {
      const message =
        nextError instanceof Error
          ? nextError.message
          : "Couldn't load this pull request.";
      if (detailsByUrl[item.url])
        setNotice(`Showing saved pull request details. ${message}`);
      else setError(message);
    } finally {
      detailsRequestsRef.current.delete(item.url);
      setDetailsRefreshing((current) => {
        const next = new Set(current);
        next.delete(item.url);
        return next;
      });
    }
  }

  async function runMutation(
    action: Extract<
      WorkspaceGitAction,
      "pr_ready" | "pr_comment" | "pr_review" | "pr_merge"
    >,
    options: {
      body?: string;
      review_action?: ReviewAction;
      merge_method?: MergeMethod;
      expected_head?: string;
    } = {},
  ) {
    if (!selected || !details) return;
    setActionBusy(action);
    setError("");
    setNotice("");
    try {
      const result = await runWorkspaceGitAction(action, {
        repository: selected.repository,
        pull_request: selected.number,
        ...options,
      });
      if (!result.ok) throw new Error(result.message);
      setNotice(result.message);
      setComment("");
      setReviewAction(null);
      setReviewBody("");
      setMergeMethod(null);
      if (action === "pr_merge") {
        setItems((current) =>
          current.filter((item) => item.url !== selected.url),
        );
        setDetailsByUrl((current) => {
          const next = { ...current };
          delete next[selected.url];
          return next;
        });
        setSelectedUrl("");
        await refreshList(false, true);
      } else await refreshDetails(selected);
    } catch (nextError) {
      setError(
        nextError instanceof Error
          ? nextError.message
          : "Couldn't update this pull request.",
      );
    } finally {
      setActionBusy(null);
    }
  }

  const layoutStyle = {
    "--pull-requests-list-width": `${listResize.size}px`,
  } as CSSProperties;
  const readiness = details ? pullRequestReadiness(details) : null;
  const mutable = details?.state.toUpperCase() === "OPEN";
  const reviews = details?.latestReviews ?? [];
  const comments = details?.comments ?? [];
  const checks = details?.checks ?? [];
  const passedChecks = checks.filter((check) => check.bucket === "pass").length;
  const reviewers = (details?.reviewRequests ?? [])
    .map((reviewer) => reviewer.login || reviewer.name)
    .filter(Boolean)
    .join(", ");

  return (
    <SheetDialog
      title="Pull requests"
      className="sheet pull-requests-sheet"
      testId="pull-requests-manager"
      resizable={{ id: "pullRequests", testId: "pull-requests-resize-handle" }}
      onClose={onClose}
    >
      <div ref={layoutRef} className="pull-requests-layout" style={layoutStyle}>
        <aside className="pull-requests-list-pane">
          <header className="pull-requests-list-header">
            <nav aria-label="Pull request filters">
              {(["all", "reviewing", "authored"] as PullRequestFilter[]).map(
                (value) => (
                  <button
                    key={value}
                    type="button"
                    className={filter === value ? "active" : ""}
                    aria-current={filter === value ? "page" : undefined}
                    onClick={() => setFilter(value)}
                  >
                    {titleCase(value)}
                  </button>
                ),
              )}
            </nav>
            <button
              className={`icon-btn${listRefreshing ? " pull-request-refreshing" : ""}`}
              type="button"
              title="Refresh pull requests"
              aria-label="Refresh pull requests"
              disabled={listLoading}
              onClick={() => void refreshList()}
            >
              <Refresh size={14} />
            </button>
          </header>

          <label className="pull-requests-search">
            <Search size={14} aria-hidden="true" />
            <input
              value={query}
              placeholder="Search pull requests"
              aria-label="Search pull requests"
              onChange={(event) => setQuery(event.currentTarget.value)}
            />
          </label>

          <div className="pull-requests-list" role="listbox" aria-label="Pull requests">
            {visibleItems.map((item) => (
              <button
                key={item.url}
                type="button"
                role="option"
                aria-selected={item.url === selectedUrl}
                className={item.url === selectedUrl ? "active" : ""}
                onPointerEnter={() => {
                  if (!detailsByUrl[item.url]) void refreshDetails(item);
                }}
                onFocus={() => {
                  if (!detailsByUrl[item.url]) void refreshDetails(item);
                }}
                onClick={() => {
                  setSelectedUrl(item.url);
                  setTab("summary");
                  setNotice("");
                }}
              >
                <Actor actor={item.author} />
                <span>
                  <strong>{item.title}</strong>
                  <small>
                    {item.repository} <b>#{item.number}</b>
                  </small>
                </span>
                <span className="pull-request-list-stats">
                  <time>{relativeDate(item.updatedAt)}</time>
                  <small>{item.commentsCount ? `${item.commentsCount} comments` : ""}</small>
                </span>
              </button>
            ))}
            {!listLoading && !visibleItems.length && (
              <div className="pull-requests-empty">
                <GitPullRequest size={24} />
                <strong>{errorPresentation?.title ?? "No pull requests"}</strong>
                <span>
                  {errorPresentation?.message ?? (query.trim()
                    ? "No pull requests match this search."
                    : `No ${filter === "all" ? "authored or review-requested" : filter} pull requests are open.`)}
                </span>
              </div>
            )}
            {listLoading && (
              <div className="pull-requests-list-skeleton" aria-label="Loading pull requests">
                {[0, 1, 2, 3].map((item) => (
                  <span key={item}>
                    <i />
                    <b />
                    <small />
                  </span>
                ))}
              </div>
            )}
          </div>
        </aside>

        <PaneResizeHandle
          resize={listResize}
          className="pull-requests-divider"
          data-testid="pull-requests-divider"
        />

        <main className="pull-request-detail-pane">
          <header className="pull-request-detail-toolbar">
            <nav aria-label="Pull request views">
              {(["summary", "timeline", "code"] as PullRequestTab[]).map(
                (value) => (
                  <button
                    key={value}
                    type="button"
                    className={tab === value ? "active" : ""}
                    aria-current={tab === value ? "page" : undefined}
                    disabled={!details}
                    onClick={() => setTab(value)}
                  >
                    {titleCase(value)}
                  </button>
                ),
              )}
            </nav>
            <div>
              {mutable && details?.isDraft && (
                <button
                  className="btn-ghost compact"
                  type="button"
                  disabled={Boolean(actionBusy)}
                  onClick={() => void runMutation("pr_ready")}
                >
                  Mark ready
                </button>
              )}
              {mutable && !details?.isDraft && (
                <>
                  <button
                    className="btn-ghost compact"
                    type="button"
                    disabled={Boolean(actionBusy)}
                    onClick={() =>
                      setReviewAction((current) => current ?? "approve")
                    }
                  >
                    Review
                  </button>
                  <button
                    className="btn-accent compact"
                    type="button"
                    disabled={!readiness?.canMerge || Boolean(actionBusy)}
                    title={readiness?.canMerge ? "Merge pull request" : readiness?.label}
                    onClick={() => setMergeMethod((current) => current ?? "squash")}
                  >
                    Merge
                  </button>
                </>
              )}
              <button
                className={`icon-btn${detailsLoading ? " pull-request-refreshing" : ""}`}
                type="button"
                title="Refresh pull request"
                aria-label="Refresh pull request"
                disabled={!selected || detailsLoading}
                onClick={() => void refreshDetails()}
              >
                <Refresh size={14} />
              </button>
              <button
                className="icon-btn"
                type="button"
                title="Open pull request on GitHub"
                aria-label="Open pull request on GitHub"
                disabled={!selected}
                onClick={() => selected && void openExternalUrl(selected.url)}
              >
                <ExternalLink size={14} />
              </button>
              <button
                className="icon-btn"
                type="button"
                title="Close"
                aria-label="Close pull requests"
                onClick={onClose}
              >
                <X size={15} />
              </button>
            </div>
          </header>

          {details && (
            <div className="pull-request-detail-scroll">
              <section className="pull-request-hero">
                <span>{selected?.repository} · PR #{details.number}</span>
                <h2>{details.title}</h2>
                <div>
                  <Actor actor={details.author} />
                  <strong>{details.author?.login || "Unknown author"}</strong>
                  <span>·</span>
                  <span>{relativeDate(details.updatedAt)}</span>
                  <span>·</span>
                  <span className={`git-pr-state ${readiness?.tone ?? ""}`}>
                    {details.isDraft ? "Draft" : readiness?.label}
                  </span>
                </div>
              </section>

              <section className="pull-request-meta">
                <div>
                  <GitBranch size={14} />
                  <span>Branch</span>
                  <strong>
                    {details.headRefName}
                    <ArrowRight size={11} />
                    {details.baseRefName}
                  </strong>
                  <small>
                    <b>+{details.additions}</b> <i>-{details.deletions}</i>
                  </small>
                </div>
                <div>
                  <Check size={14} />
                  <span>Reviewers</span>
                  <strong>{reviewers || "No reviewers"}</strong>
                </div>
                <div>
                  <GitPullRequest size={14} />
                  <span>Comments</span>
                  <strong>{comments.length + reviews.length}</strong>
                </div>
                <div>
                  <Check size={14} />
                  <span>Checks</span>
                  <strong>
                    {checks.length ? `${passedChecks}/${checks.length} passing` : "No checks"}
                  </strong>
                </div>
              </section>

              {reviewAction && (
                <section className="pull-request-action-card">
                  <div>
                    <strong>Submit review</strong>
                    <button type="button" aria-label="Cancel review" title="Cancel review" onClick={() => setReviewAction(null)}>
                      <X size={13} />
                    </button>
                  </div>
                  <select
                    aria-label="Review action"
                    value={reviewAction}
                    onChange={(event) =>
                      setReviewAction(event.currentTarget.value as ReviewAction)
                    }
                  >
                    <option value="approve">Approve</option>
                    <option value="request_changes">Request changes</option>
                    <option value="comment">Comment</option>
                  </select>
                  <textarea
                    aria-label="Review note"
                    value={reviewBody}
                    rows={3}
                    placeholder={reviewAction === "approve" ? "Optional review note" : "Review note"}
                    onChange={(event) => setReviewBody(event.currentTarget.value)}
                  />
                  <button
                    className="btn-accent"
                    type="button"
                    disabled={
                      Boolean(actionBusy) ||
                      (reviewAction !== "approve" && !reviewBody.trim())
                    }
                    onClick={() =>
                      void runMutation("pr_review", {
                        body: reviewBody.trim(),
                        review_action: reviewAction,
                      })
                    }
                  >
                    Submit review
                  </button>
                </section>
              )}

              {mergeMethod && (
                <section className="pull-request-action-card">
                  <div>
                    <strong>Merge pull request</strong>
                    <button type="button" aria-label="Cancel merge" title="Cancel merge" onClick={() => setMergeMethod(null)}>
                      <X size={13} />
                    </button>
                  </div>
                  <select
                    aria-label="Merge method"
                    value={mergeMethod}
                    onChange={(event) =>
                      setMergeMethod(event.currentTarget.value as MergeMethod)
                    }
                  >
                    <option value="squash">Squash and merge</option>
                    <option value="merge">Create a merge commit</option>
                    <option value="rebase">Rebase and merge</option>
                  </select>
                  <button
                    className="btn-accent"
                    type="button"
                    disabled={Boolean(actionBusy)}
                    onClick={() =>
                      void runMutation("pr_merge", {
                        merge_method: mergeMethod,
                        expected_head: details.headRefOid,
                      })
                    }
                  >
                    Confirm merge
                  </button>
                </section>
              )}

              {tab === "summary" && (
                <>
                  <details className="git-pr-section" open>
                    <summary>Description</summary>
                    <div className="git-pr-markdown">
                      {details.body.trim() ? (
                        <Suspense fallback={<p>Loading...</p>}>
                          <Markdown content={details.body} collapseArtifacts={false} allowHtml />
                        </Suspense>
                      ) : (
                        <p>No description.</p>
                      )}
                    </div>
                  </details>
                  <details className="git-pr-section" open>
                    <summary>
                      Checks <span>{checks.length}</span>
                    </summary>
                    <div className="git-pr-list">
                      {checks.map((check) => (
                        <button
                          type="button"
                          key={`${check.workflow}:${check.name}`}
                          className={`git-pr-check ${check.bucket}`}
                          disabled={!check.link}
                          onClick={() => check.link && void openExternalUrl(check.link)}
                        >
                          <span className="git-pr-check-dot" aria-hidden="true" />
                          <strong>{check.name}</strong>
                          <small>{check.workflow || check.state}</small>
                          <span>{check.bucket}</span>
                        </button>
                      ))}
                      {!checks.length && (
                        <p>{details.checksError || "No checks reported."}</p>
                      )}
                    </div>
                  </details>
                  <details className="git-pr-section" open>
                    <summary>
                      Comments <span>{comments.length + reviews.length}</span>
                    </summary>
                    {renderConversation()}
                  </details>
                </>
              )}

              {tab === "timeline" && (
                <section className="pull-request-tab-section">
                  <h3>Timeline</h3>
                  {renderConversation()}
                </section>
              )}

              {tab === "code" && (
                <section className="pull-request-tab-section">
                  <h3>
                    Changed files <span>{details.changedFiles}</span>
                  </h3>
                  <div className="pull-request-files">
                    {(details.files ?? []).map((file) => (
                      <div key={file.path}>
                        <Code size={14} />
                        <strong>{file.path}</strong>
                        <span>
                          <b>+{file.additions}</b> <i>-{file.deletions}</i>
                        </span>
                      </div>
                    ))}
                    {!details.files?.length && <p>No changed files reported.</p>}
                  </div>
                </section>
              )}

              {(error || notice) && (
                <div
                  className={`pull-requests-notice${error ? " error" : ""}`}
                  role={error ? "alert" : "status"}
                >
                  {error || notice}
                </div>
              )}
            </div>
          )}

          {!details && (
            <div className="pull-requests-empty pull-request-detail-empty">
              {selected && detailsLoading ? (
                <div className="pull-request-detail-skeleton" aria-label="Loading pull request">
                  <i />
                  <strong />
                  <strong />
                  <span />
                  <span />
                  <span />
                </div>
              ) : (
                <>
                  <GitPullRequest size={28} />
                  <strong>{errorPresentation?.title ?? "Select a pull request"}</strong>
                  <span>{errorPresentation?.message ?? "Review details, checks, comments, and changed files here."}</span>
                </>
              )}
              {errorPresentation && (
                <div className="pull-request-error-actions" role="alert">
                  <button className="btn-accent" type="button" onClick={() => void refreshList()}>
                    Retry
                  </button>
                  {errorPresentation.helpUrl && (
                    <button className="btn-ghost" type="button" onClick={() => void openExternalUrl(errorPresentation.helpUrl!)}>
                      {errorPresentation.helpLabel}
                    </button>
                  )}
                  <details>
                    <summary>Technical details</summary>
                    <code>{errorPresentation.detail}</code>
                  </details>
                </div>
              )}
            </div>
          )}
        </main>
      </div>

    </SheetDialog>
  );

  function renderConversation() {
    return (
      <div className="git-pr-conversation">
        {reviews.map((review, index) => (
          <article key={`review:${review.author?.login}:${index}`}>
            <header>
              <span>
                <Actor actor={review.author} />
                <strong>{review.author?.login || "Reviewer"}</strong>
              </span>
              <span>{titleCase(review.state)}</span>
            </header>
            {review.body?.trim() && (
              <Suspense fallback={<p>Loading...</p>}>
                <Markdown content={review.body} collapseArtifacts={false} allowHtml />
              </Suspense>
            )}
          </article>
        ))}
        {comments.map((item, index) => (
          <article key={`comment:${item.author?.login}:${index}`}>
            <header>
              <span>
                <Actor actor={item.author} />
                <strong>{item.author?.login || "Comment"}</strong>
              </span>
              <span>{item.createdAt ? new Date(item.createdAt).toLocaleString() : ""}</span>
            </header>
            <Suspense fallback={<p>Loading...</p>}>
              <Markdown content={item.body} collapseArtifacts={false} allowHtml />
            </Suspense>
          </article>
        ))}
        {!comments.length && !reviews.length && <p>No comments.</p>}
        {mutable && (
          <div className="git-pr-comment">
            <textarea
              aria-label="Comment"
              value={comment}
              placeholder="Leave a comment"
              rows={3}
              onChange={(event) => setComment(event.currentTarget.value)}
            />
            <button
              type="button"
              disabled={!comment.trim() || Boolean(actionBusy)}
              onClick={() =>
                void runMutation("pr_comment", { body: comment.trim() })
              }
            >
              Comment
            </button>
          </div>
        )}
      </div>
    );
  }
}
