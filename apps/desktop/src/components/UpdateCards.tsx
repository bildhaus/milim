import { useEffect, useState, type CSSProperties } from "react";
import { shouldShowUpdateCards } from "../update/service";
import { useUpdateStore } from "../update/store";
import releaseFeed from "../update/releases.json";
import { ArrowLeft, ArrowRight, Check, FileText, GitHub, GitPullRequest, Plug, X } from "./icons";
import { SheetDialog } from "./SheetDialog";
import "./UpdateCards.css";

const SEEN_VERSION_KEY = "milim.local.update-cards.seen-version";
const PREVIEW_EVENT = "milim:update-cards-preview";
const ICONS = {
  "file-text": FileText,
  github: GitHub,
  "git-pull-request": GitPullRequest,
  plug: Plug,
};

type Release = {
  version: string;
  summary: string;
  items: Array<{
    id: string;
    eyebrow: string;
    title: string;
    description: string;
    details: string[];
    accent: string;
    icon: keyof typeof ICONS | "google";
  }>;
};
const RELEASES = releaseFeed as Record<string, Release>;

type CardStyle = CSSProperties & {
  "--update-card-accent": string;
  "--update-stack-position": number;
};

function seenVersion(): string | null {
  try {
    return localStorage.getItem(SEEN_VERSION_KEY);
  } catch {
    return null;
  }
}

export function showUpdateCardsForDebug() {
  window.dispatchEvent(new Event(PREVIEW_EVENT));
}

export function UpdateCards() {
  const currentVersion = useUpdateStore((state) => state.currentVersion);
  const loadCurrentVersion = useUpdateStore((state) => state.loadCurrentVersion);
  const [open, setOpen] = useState(false);
  const [index, setIndex] = useState(0);
  const release = currentVersion ? RELEASES[currentVersion] : undefined;
  const item = release?.items[index];
  const Icon = item && item.icon !== "google" ? ICONS[item.icon] : FileText;

  useEffect(() => {
    void loadCurrentVersion();
  }, [loadCurrentVersion]);

  useEffect(() => {
    if (release && shouldShowUpdateCards(currentVersion, release.version, seenVersion())) {
      setOpen(true);
    }
  }, [currentVersion, release]);

  useEffect(() => {
    const showPreview = () => {
      if (!release) return;
      setIndex(0);
      setOpen(true);
    };
    window.addEventListener(PREVIEW_EVENT, showPreview);
    return () => window.removeEventListener(PREVIEW_EVENT, showPreview);
  }, [release]);

  if (!open || !release || !item) return null;
  const activeRelease = release;

  function dismiss() {
    try {
      localStorage.setItem(SEEN_VERSION_KEY, activeRelease.version);
    } catch {
      // A blocked storage write should not trap the user in the dialog.
    }
    setOpen(false);
  }

  function move(step: -1 | 1) {
    setIndex((current) => (current + step + activeRelease.items.length) % activeRelease.items.length);
  }

  return (
    <SheetDialog
      title={`What's new in Milim ${release.version}`}
      className="sheet update-cards-dialog"
      overlayClassName="sheet-overlay update-cards-overlay"
      testId="update-cards"
      onClose={dismiss}
    >
      <div className="update-cards-stage" aria-live="polite">
        {release.items.map((card, cardIndex) => {
          const position = (cardIndex - index + release.items.length) % release.items.length;
          const active = cardIndex === index;
          const style: CardStyle = {
            "--update-card-accent": card.accent,
            "--update-stack-position": position,
            zIndex: release.items.length - position,
          };

          return (
            <article
              key={card.id}
              className="update-card"
              style={style}
              data-active={active ? "true" : "false"}
              aria-hidden={!active}
              onClick={active ? undefined : () => setIndex(cardIndex)}
            >
              {active && (
                <div className="update-card-content">
                  <header className="update-card-header">
                    <span>What&apos;s new</span>
                    <span>v{release.version} · {index + 1}/{release.items.length}</span>
                    <button
                      className="icon-btn update-cards-close"
                      type="button"
                      aria-label="Dismiss updates"
                      onClick={dismiss}
                    >
                      <X size={15} />
                    </button>
                  </header>

                  <div className="update-card-body">
                    <div className="update-card-visual" aria-hidden="true">
                      <div className="update-card-rays">
                        {Array.from({ length: 64 }, (_, rayIndex) => (
                          <span
                            key={rayIndex}
                            style={{ "--update-ray-index": rayIndex } as CSSProperties}
                          />
                        ))}
                      </div>
                      {item.icon === "google"
                        ? <span className="update-card-google-logo" />
                        : <Icon size={48} />}
                    </div>

                    <div className="update-card-copy">
                      <p className="update-card-eyebrow">{item.eyebrow}</p>
                      <h2>{item.title}</h2>
                      <p className="update-card-description">{item.description}</p>
                      <ul className="update-card-details">
                        {item.details.map((detail) => (
                          <li key={detail}>
                            <Check size={13} aria-hidden="true" />
                            <span>{detail}</span>
                          </li>
                        ))}
                      </ul>
                    </div>
                  </div>

                  <footer className="update-card-footer">
                    <span className="update-card-progress">
                      {index + 1} of {release.items.length}
                    </span>
                    <div className="update-card-actions">
                      {index > 0 && (
                        <button className="btn-ghost" type="button" onClick={() => move(-1)}>
                          <ArrowLeft size={14} aria-hidden="true" />
                          Back
                        </button>
                      )}
                      <button
                        className="btn-accent update-card-next"
                        type="button"
                        onClick={index === release.items.length - 1 ? dismiss : () => move(1)}
                      >
                        {index === release.items.length - 1 ? "Done" : "Next"}
                        {index === release.items.length - 1
                          ? <Check size={14} aria-hidden="true" />
                          : <ArrowRight size={14} aria-hidden="true" />}
                      </button>
                    </div>
                  </footer>
                </div>
              )}
            </article>
          );
        })}
      </div>
    </SheetDialog>
  );
}
