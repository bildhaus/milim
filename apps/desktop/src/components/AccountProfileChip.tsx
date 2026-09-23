import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  AUTO_ACCOUNT_PROFILE_ID,
  DEFAULT_ACCOUNT_PROFILE_ID,
  listAccountProfiles,
  type AccountProfile,
} from "../api";
import { accountProfileRuntimeForModel } from "../lib/accountProfiles";
import { formatCooldown, peakUsagePercent } from "./AccountProfiles";
import { Check, ChevronDown } from "./icons";
import { ProviderIcon } from "./ProviderIcon";

/** Kept in sync with `.account-profile-menu` width in chat.css. */
const MENU_WIDTH = 300;

function optionDetail(profile: AccountProfile, now: number): string {
  const cooldown = formatCooldown(profile.cooled_until_ms, now);
  if (cooldown) return `Rate limited · resets in ${cooldown}`;
  const usage = peakUsagePercent(profile);
  if (usage != null) return `${Math.round(usage)}% used`;
  return profile.is_default ? "The CLI's own folder" : "No usage reported yet";
}

/**
 * Which signed-in account this chat's next turn uses. Hidden until a second
 * account exists, so single-account installs see no extra control.
 */
export function AccountProfileChip({
  model,
  selected,
  onSelect,
}: {
  model: string;
  /** `auto`, a profile id, or undefined for the runtime's own account. */
  selected: string | undefined;
  onSelect: (profileId: string | undefined) => void;
}) {
  const runtime = accountProfileRuntimeForModel(model);
  const [profiles, setProfiles] = useState<AccountProfile[]>([]);
  const [autoSelection, setAutoSelection] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  const [position, setPosition] = useState<{ left: number; top: number }>();
  const wrapRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!runtime) {
      setProfiles([]);
      return;
    }
    const controller = new AbortController();
    void listAccountProfiles(runtime, controller.signal)
      .then((result) => {
        setProfiles(result.profiles);
        setAutoSelection(result.auto_selection);
      })
      .catch(() => {
        // A chat is still usable without this chip; the Providers panel is
        // where account problems are reported.
      });
    return () => controller.abort();
  }, [runtime, open]);

  useEffect(() => {
    if (!open) return;
    setNow(Date.now());
    const closeOnOutside = (event: MouseEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      // The menu is portalled out of the chip, so it is not a DOM descendant.
      if (wrapRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      setOpen(false);
    };
    const close = () => setOpen(false);
    document.addEventListener("mousedown", closeOnOutside);
    window.addEventListener("resize", close);
    window.addEventListener("scroll", close, true);
    return () => {
      document.removeEventListener("mousedown", closeOnOutside);
      window.removeEventListener("resize", close);
      window.removeEventListener("scroll", close, true);
    };
  }, [open]);

  const current = useMemo(() => {
    if (selected === AUTO_ACCOUNT_PROFILE_ID) return null;
    const id = selected ?? DEFAULT_ACCOUNT_PROFILE_ID;
    return profiles.find((profile) => profile.id === id) ?? null;
  }, [profiles, selected]);

  if (!runtime || profiles.length < 2) return null;

  const isAuto = selected === AUTO_ACCOUNT_PROFILE_ID;
  const autoProfile = profiles.find((profile) => profile.id === autoSelection);
  const label = isAuto ? "Auto" : (current?.label ?? "Default");
  const detail = isAuto
    ? (autoProfile?.label ?? "")
    : (formatCooldown(current?.cooled_until_ms, now) ?? "");

  function choose(profileId: string | undefined) {
    setOpen(false);
    onSelect(profileId);
  }

  /// The chip sits inside clipped, lower-stacked composer chrome, so the menu
  /// is portalled to the body and positioned against the trigger instead.
  function toggleMenu() {
    if (open) {
      setOpen(false);
      return;
    }
    const rect = triggerRef.current?.getBoundingClientRect();
    if (rect) {
      const width = MENU_WIDTH;
      const estimatedHeight = 108 + profiles.length * 44;
      const left = Math.max(8, Math.min(window.innerWidth - width - 8, rect.right - width));
      const top =
        rect.top - estimatedHeight - 6 >= 8 ? rect.top - estimatedHeight - 6 : rect.bottom + 6;
      setPosition({ left, top });
    }
    setOpen(true);
  }

  return (
    <div className="chip-wrap" ref={wrapRef}>
      <button
        ref={triggerRef}
        type="button"
        className={"chip" + (current && formatCooldown(current.cooled_until_ms, now) ? " chip-warn" : "")}
        data-testid="account-profile-chip"
        onClick={toggleMenu}
        title={
          isAuto
            ? `Auto picks the ${runtime} account with the most room left${autoProfile ? `; currently ${autoProfile.label}` : ""}`
            : `This chat uses the ${label} ${runtime} account`
        }
        aria-label={`Account for this chat, ${label}`}
        aria-haspopup="menu"
        aria-expanded={open}
      >
        <ProviderIcon brand={runtime} size={13} />
        <span className="chip-label">{label}</span>
        {detail && <span className="chip-detail">{detail}</span>}
        <ChevronDown size={12} className="chip-chev" />
      </button>
      {open && createPortal(
        <div
          ref={menuRef}
          className="context-menu account-profile-menu message-popover-layer"
          role="menu"
          aria-label={`Account for this chat`}
          data-native-preview-blocker="true"
          style={position}
        >
          <button
            className={"context-row" + (isAuto ? " context-on" : "")}
            type="button"
            role="menuitemradio"
            aria-checked={isAuto}
            onClick={() => choose(AUTO_ACCOUNT_PROFILE_ID)}
          >
            <span className="context-icon">{isAuto && <Check size={14} />}</span>
            <span className="context-text">
              <strong>Auto</strong>
              <span>
                {autoProfile
                  ? `Uses ${autoProfile.label} next; re-picks each turn`
                  : "Picks the account with the most room left"}
              </span>
            </span>
          </button>
          {profiles.map((profile) => {
            const active = !isAuto && (selected ?? DEFAULT_ACCOUNT_PROFILE_ID) === profile.id;
            return (
              <button
                key={profile.id}
                className={"context-row" + (active ? " context-on" : "")}
                type="button"
                role="menuitemradio"
                aria-checked={active}
                data-testid={`account-profile-option-${profile.id}`}
                onClick={() =>
                  choose(profile.is_default ? undefined : profile.id)
                }
              >
                <span className="context-icon">{active && <Check size={14} />}</span>
                <span className="context-text">
                  <strong>{profile.label}</strong>
                  <span>{optionDetail(profile, now)}</span>
                </span>
              </button>
            );
          })}
          <p className="account-profile-menu-note">
            Switching accounts starts a fresh {runtime} session for this chat.
            milim replays the conversation so far; the other account keeps its
            own session.
          </p>
        </div>,
        document.body,
      )}
    </div>
  );
}
