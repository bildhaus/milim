import { useCallback, useEffect, useMemo, useState } from "react";
import {
  decideMobileCompanionPairingRequest,
  getMobileCompanionStatus,
  type MobileCompanionPairingRequest,
} from "../api";
import { useUiPreferences } from "../ui/store";
import { Shield, Smartphone } from "./icons";

function platformLabel(platform: MobileCompanionPairingRequest["platform"]): string {
  if (platform === "android") return "Android";
  if (platform === "ios") return "iPhone or iPad";
  return "Mobile device";
}

function expiryLabel(expiresAt: number): string {
  const seconds = Math.max(0, expiresAt - Math.floor(Date.now() / 1000));
  if (seconds < 60) return `Expires in ${seconds}s`;
  return `Expires in ${Math.ceil(seconds / 60)}m`;
}

export function MobilePairingApprovalPrompt() {
  const [requests, setRequests] = useState<MobileCompanionPairingRequest[]>([]);
  const [busyId, setBusyId] = useState<string | null>(null);
  const pushNotice = useUiPreferences((state) => state.pushNotice);
  const refresh = useCallback(async () => {
    if (document.visibilityState === "hidden") return;
    try {
      const status = await getMobileCompanionStatus();
      setRequests(status.pairing_requests ?? []);
    } catch {
      // The local service may still be starting; the next visible poll retries.
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 1_200);
    const onVisibilityChange = () => void refresh();
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, [refresh]);

  const request = useMemo(
    () => requests
      .filter((item) => item.status === "pending")
      .sort((left, right) => left.created_at - right.created_at)[0] ?? null,
    [requests],
  );
  if (!request) return null;

  async function decide(approved: boolean) {
    setBusyId(request!.id);
    try {
      const status = await decideMobileCompanionPairingRequest(request!.id, approved);
      setRequests(status.pairing_requests ?? []);
      pushNotice({
        tone: approved ? "success" : "info",
        message: approved
          ? `${request!.device_name} can finish connecting.`
          : `Connection request from ${request!.device_name} declined.`,
      });
    } catch (error) {
      pushNotice({
        tone: "error",
        message: `Pairing decision failed: ${error instanceof Error ? error.message : String(error)}`,
      });
      await refresh();
    } finally {
      setBusyId(null);
    }
  }

  const busy = busyId === request.id;
  const queued = requests.filter((item) => item.status === "pending").length - 1;
  return (
    <div className="git-modal-backdrop mobile-pair-request-backdrop" data-native-preview-blocker="true">
      <section
        className="git-modal mobile-pair-request-modal"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="mobile-pair-request-title"
        aria-describedby="mobile-pair-request-description"
        data-testid="mobile-pair-request-prompt"
      >
        <div className="mobile-pair-request-icon" aria-hidden="true">
          <Smartphone size={20} />
        </div>
        <div className="mobile-pair-request-copy">
          <span className="mobile-pair-request-eyebrow">Nearby device</span>
          <h2 id="mobile-pair-request-title">Connect {request.device_name}?</h2>
          <p id="mobile-pair-request-description">
            This {platformLabel(request.platform).toLowerCase()} wants to control chats, runs, and approvals on this desktop.
          </p>
        </div>
        <div className="mobile-pair-request-security">
          <Shield size={15} />
          <span>Approve only a device you recognize. You can revoke it anytime in Settings, under Mobile.</span>
        </div>
        <div className="mobile-pair-request-meta">
          <span>{platformLabel(request.platform)}</span>
          <span>{expiryLabel(request.expires_at)}</span>
          {queued > 0 ? <span>+{queued} waiting</span> : null}
        </div>
        <div className="mobile-pair-request-actions">
          <button type="button" disabled={busy} onClick={() => void decide(false)}>
            Not now
          </button>
          <button type="button" className="primary" disabled={busy} onClick={() => void decide(true)} autoFocus>
            {busy ? "Approving..." : "Allow device"}
          </button>
        </div>
      </section>
    </div>
  );
}
