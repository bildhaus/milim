import { useEffect, useMemo, useState } from "react";
import QRCode from "qrcode";
import {
  apiBaseUrl,
  configureMobileTailscale,
  getControlBootstrap,
  getMobileCompanionStatus,
  getMobileLanStatus,
  mobileTailscaleStatus,
  openExternalUrl,
  revokeMobileCompanionDevice,
  setMobileCompanionEnabled,
  setMobileLanEnabled,
  startMobileCompanionPairing,
  type MobileCompanionStatus,
  type MobileLanStatus,
  type MobileTailscaleStatus,
} from "../api";
import { readUserStateKey, writeUserStateKey } from "../persistence/userStateStorage";
import { Copy, Globe, Refresh, Smartphone } from "./icons";
import { describeMobileTailscale, type MobileTailscaleAction } from "./mobileTailscaleStatus";
import { Toggle } from "./ui";

const MOBILE_URL_BASE_KEY = "milim.mobile.urlBase";
const TAILSCALE_DOWNLOAD_URL = "https://tailscale.com/download";

function normalizeBase(value: string): string {
  return value.trim().replace(/\/+$/, "");
}

function formatTime(seconds?: number | null): string {
  if (!seconds) return "Never";
  return new Date(seconds * 1000).toLocaleString();
}

function emulatorBaseFromApiBase(base: string): string {
  try {
    const url = new URL(base);
    if (url.hostname === "127.0.0.1" || url.hostname === "localhost") {
      url.hostname = "10.0.2.2";
      return url.toString().replace(/\/+$/, "");
    }
  } catch {
    // Fall through to the original base.
  }
  return base;
}

export function MobileCompanionSettings() {
  const [status, setStatus] = useState<MobileCompanionStatus | null>(null);
  const [lan, setLan] = useState<MobileLanStatus | null>(null);
  const [hostId, setHostId] = useState("");
  const [urlBase, setUrlBase] = useState("");
  const [qrDataUrl, setQrDataUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [tailscaleBusy, setTailscaleBusy] = useState(false);
  const [tailscale, setTailscale] = useState<MobileTailscaleStatus | null>(null);
  const [tailscaleChecking, setTailscaleChecking] = useState(true);
  const [tailscaleError, setTailscaleError] = useState<string | null>(null);
  const [notice, setNotice] = useState<{ tone: "info" | "error" | "success"; message: string } | null>(null);

  useEffect(() => {
    let alive = true;
    void (async () => {
      try {
        const [nextStatus, nextLan, bootstrap, base, savedBase] = await Promise.all([
          getMobileCompanionStatus(),
          getMobileLanStatus(),
          getControlBootstrap(),
          apiBaseUrl(),
          Promise.resolve(readUserStateKey(MOBILE_URL_BASE_KEY)).catch(() => null),
        ]);
        if (!alive) return;
        setStatus(nextStatus);
        setLan(nextLan);
        setHostId(bootstrap.host_id);
        setUrlBase(savedBase || emulatorBaseFromApiBase(base));
      } catch (error) {
        if (alive) setNotice({ tone: "error", message: `Mobile companion unavailable: ${error instanceof Error ? error.message : String(error)}` });
      }
    })();
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    let alive = true;
    const checkTailscale = () => {
      setTailscaleChecking(true);
      void mobileTailscaleStatus()
        .then((next) => {
          if (!alive) return;
          setTailscale(next);
          setTailscaleError(null);
        })
        .catch((error) => {
          if (!alive) return;
          setTailscaleError(error instanceof Error ? error.message : String(error));
        })
        .finally(() => {
          if (alive) setTailscaleChecking(false);
        });
    };
    checkTailscale();
    window.addEventListener("focus", checkTailscale);
    return () => {
      alive = false;
      window.removeEventListener("focus", checkTailscale);
    };
  }, []);

  useEffect(() => {
    let alive = true;
    const timer = window.setInterval(() => {
      void getMobileCompanionStatus()
        .then((nextStatus) => {
          if (alive) setStatus(nextStatus);
        })
        .catch(() => {});
    }, 2_000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, []);

  const pairingUrl = useMemo(() => {
    if (!status?.pairing || !urlBase.trim()) return "";
    return `${normalizeBase(urlBase)}${status.pairing.path}`;
  }, [status?.pairing, urlBase]);

  const nativePairingUrl = useMemo(() => {
    const pairingHostId = hostId || lan?.host_id || "";
    if (!status?.pairing || !pairingUrl || !pairingHostId) return "";
    const query = new URLSearchParams({
      endpoint: normalizeBase(urlBase),
      pair_id: status.pairing.id,
      secret: new URL(pairingUrl).searchParams.get("secret") ?? "",
      host_id: pairingHostId,
    });
    return `milim://pair?${query}`;
  }, [hostId, lan?.host_id, pairingUrl, status?.pairing, urlBase]);

  useEffect(() => {
    let alive = true;
    if (!nativePairingUrl) {
      setQrDataUrl("");
      return;
    }
    void QRCode.toDataURL(nativePairingUrl, { margin: 1, width: 220 })
      .then((dataUrl) => {
        if (alive) setQrDataUrl(dataUrl);
      })
      .catch(() => {
        if (alive) setQrDataUrl("");
      });
    return () => {
      alive = false;
    };
  }, [nativePairingUrl]);

  async function refresh() {
    setBusy(true);
    setTailscaleChecking(true);
    try {
      const [nextStatus, nextLan, nextTailscale] = await Promise.all([
        getMobileCompanionStatus(),
        getMobileLanStatus(),
        mobileTailscaleStatus()
          .then((value) => ({ value, error: null as string | null }))
          .catch((error) => ({
            value: null,
            error: error instanceof Error ? error.message : String(error),
          })),
      ]);
      setStatus(nextStatus);
      setLan(nextLan);
      if (nextTailscale.value) {
        setTailscale(nextTailscale.value);
        setTailscaleError(null);
        setNotice({ tone: "success", message: "Connection status refreshed." });
      } else {
        setTailscaleError(nextTailscale.error);
        setNotice({ tone: "info", message: "Nearby status refreshed, but Tailscale could not be checked." });
      }
    } catch (error) {
      setNotice({ tone: "error", message: `Refresh failed: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setTailscaleChecking(false);
      setBusy(false);
    }
  }

  async function updateEnabled(enabled: boolean) {
    setBusy(true);
    try {
      setStatus(await setMobileCompanionEnabled(enabled));
      setNotice({ tone: "success", message: enabled ? "Mobile companion bridge enabled." : "Mobile companion bridge disabled." });
    } catch (error) {
      setNotice({ tone: "error", message: `Update failed: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setBusy(false);
    }
  }

  async function updateLan(enabled: boolean) {
    setBusy(true);
    try {
      if (enabled && !status?.enabled) {
        setStatus(await setMobileCompanionEnabled(true));
      }
      setLan(await setMobileLanEnabled(enabled));
      setNotice({
        tone: enabled ? "info" : "success",
        message: enabled
          ? "LAN discovery enabled. Use this only on a trusted network; traffic is plain HTTP."
          : "LAN discovery disabled.",
      });
    } catch (error) {
      setNotice({ tone: "error", message: `LAN update failed: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setBusy(false);
    }
  }

  async function createPairing(baseValue: string, message = "QR code and pairing link ready.") {
    const pairing = await startMobileCompanionPairing();
    setStatus((current) =>
      current
        ? { ...current, enabled: true, pairing }
        : { enabled: true, pairing, pairing_requests: [], devices: [] },
    );
    const base = normalizeBase(baseValue);
    if (base) {
      await persistUrlBase(base);
      setUrlBase(base);
    }
    setNotice({ tone: "success", message });
  }

  function persistUrlBase(value: string): Promise<void> {
    return Promise.resolve(writeUserStateKey(MOBILE_URL_BASE_KEY, value)).catch(() => {});
  }

  async function startPairing() {
    setBusy(true);
    try {
      const remoteBase = tailscale?.logged_in && tailscale.serve_configured && tailscale.public_url
        ? tailscale.public_url
        : urlBase;
      await createPairing(remoteBase);
    } catch (error) {
      setNotice({ tone: "error", message: `Pairing failed: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setBusy(false);
    }
  }

  async function configureTailscaleRemoteAccess() {
    setBusy(true);
    setTailscaleBusy(true);
    setNotice({ tone: "info", message: "Setting up Tailscale Serve..." });
    try {
      if (!enabled) {
        setStatus(await setMobileCompanionEnabled(true));
      }
      const connection = await configureMobileTailscale();
      setTailscale(connection);
      setTailscaleError(null);
      if (!connection.installed) {
        await openTailscaleDownload("Tailscale is not installed. I opened the official download page. Install it, sign in, then return here.");
        return;
      }
      if (!connection.logged_in || !connection.public_url) {
        throw new Error(connection.message || "Tailscale is not ready.");
      }
      if (!connection.serve_configured) {
        throw new Error(connection.message || "Tailscale Serve did not report the mobile control target.");
      }
      const base = normalizeBase(connection.public_url);
      await persistUrlBase(base);
      setUrlBase(base);
      setNotice({
        tone: "success",
        message: connection.message || "Tailscale remote access is ready. Use Show pairing code when a phone cannot find this desktop nearby.",
      });
    } catch (error) {
      setNotice({ tone: "error", message: `Tailscale setup failed: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setTailscaleBusy(false);
      setBusy(false);
    }
  }

  async function runTailscaleAction(action: MobileTailscaleAction) {
    if (action === "install") {
      await openTailscaleDownload();
      return;
    }
    if (action === "configure") {
      await configureTailscaleRemoteAccess();
      return;
    }
    await refresh();
  }

  async function openTailscaleDownload(message = "Opened the official Tailscale download page.") {
    try {
      await openExternalUrl(TAILSCALE_DOWNLOAD_URL);
      setNotice({ tone: "info", message });
    } catch (error) {
      setNotice({ tone: "error", message: `Could not open Tailscale download: ${error instanceof Error ? error.message : String(error)}` });
    }
  }

  async function revoke(id: string) {
    setBusy(true);
    try {
      setStatus(await revokeMobileCompanionDevice(id));
      setNotice({ tone: "success", message: "Device revoked." });
    } catch (error) {
      setNotice({ tone: "error", message: `Revoke failed: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setBusy(false);
    }
  }

  async function copyPairingUrl() {
    if (!nativePairingUrl) return;
    await navigator.clipboard?.writeText(nativePairingUrl).catch(() => undefined);
    setNotice({ tone: "success", message: "Native pairing link copied." });
  }

  const enabled = Boolean(status?.enabled);
  const tailscaleView = describeMobileTailscale(tailscale, tailscaleChecking, tailscaleError);
  const nearbyReady = enabled && Boolean(lan?.active);
  const remoteReady = enabled && Boolean(tailscale?.logged_in && tailscale.serve_configured && tailscale.public_url);
  const connectionReady = nearbyReady || remoteReady;
  const connectionTitle = !enabled
    ? "Mobile access is off"
    : connectionReady
      ? "Ready to connect"
      : "Connection setup needed";
  const connectionDetail = !enabled
    ? "Turn on mobile access to connect a phone."
    : nearbyReady && remoteReady
      ? "Nearby phones can tap this desktop. Away from this network, pair once with a private QR code."
      : nearbyReady
        ? "Open milim mobile nearby and tap this desktop. Set up Tailscale only for access from elsewhere."
        : remoteReady
          ? "Remote access is ready. Pair this phone once with a private QR code."
          : "Turn on nearby discovery or enable Tailscale remote access.";
  const pairButtonLabel = status?.pairing ? "Create new code" : "Show pairing code";

  return (
    <div className="mobile-companion-settings">
      <div className={`mobile-connection-overview ${connectionReady ? "ready" : "attention"}`}>
        <div className="mobile-connection-overview-icon" aria-hidden="true"><Smartphone size={18} /></div>
        <div className="mobile-connection-overview-copy" aria-live="polite">
          <strong>{connectionTitle}</strong>
          <span>{connectionDetail}</span>
          <div className="mobile-connection-chips" aria-label="Connection availability">
            <span className={nearbyReady ? "ready" : "muted"}>
              {!enabled ? "Nearby paused" : nearbyReady ? "Nearby ready" : "Nearby off"}
            </span>
            <span className={remoteReady ? "ready" : !enabled || tailscaleView.tone === "checking" ? "muted" : "attention"}>
              {!enabled ? "Remote paused" : remoteReady ? "Remote ready" : `Remote ${tailscaleView.badge.toLowerCase()}`}
            </span>
          </div>
        </div>
        <div className="mobile-connection-overview-actions">
          {!enabled ? (
            <button className="btn-accent" type="button" disabled={busy} onClick={() => void updateEnabled(true)}>
              Turn on
            </button>
          ) : tailscaleView.action && tailscaleView.actionLabel ? (
            <button
              className="btn-ghost"
              type="button"
              disabled={tailscaleBusy || busy}
              onClick={() => void runTailscaleAction(tailscaleView.action as MobileTailscaleAction)}
              data-testid={`mobile-tailscale-${tailscaleView.action}`}
            >
              {tailscaleBusy ? "Working..." : tailscaleView.actionLabel}
            </button>
          ) : null}
          {tailscaleView.action !== "refresh" ? (
            <button
              className="btn-ghost mobile-connection-refresh"
              type="button"
              disabled={busy}
              onClick={() => void refresh()}
              title="Refresh connection status"
              aria-label="Refresh connection status"
            >
              <Refresh size={14} />
            </button>
          ) : null}
        </div>
      </div>

      {notice && (
        <p className={`mobile-companion-notice ${notice.tone}`} role={notice.tone === "error" ? "alert" : "status"}>
          {notice.message}
        </p>
      )}

      <div className="mobile-pair-device-action">
        <div>
          <strong>{status?.devices.length ? "Add another phone" : "Can't find this desktop nearby?"}</strong>
          <span>Create a private QR code or link. It expires automatically.</span>
        </div>
        <button className="btn-accent" type="button" disabled={!enabled || busy} onClick={() => void startPairing()} data-testid="mobile-companion-start-pairing">
          {pairButtonLabel}
        </button>
      </div>

      {nativePairingUrl && (
        <div className="mobile-pairing-panel">
          {qrDataUrl && <img className="mobile-pairing-qr" src={qrDataUrl} alt="Mobile companion pairing QR code" />}
          <div className="mobile-pairing-copy">
            <strong>Scan with milim mobile</strong>
            <span>Open the pairing scanner on your phone and point it at this code.</span>
            <button className="btn-ghost" type="button" onClick={() => void copyPairingUrl()}>
              <Copy size={13} /> Copy pairing link
            </button>
            <small>Expires {formatTime(status?.pairing?.expires_at)}.</small>
          </div>
        </div>
      )}

      <details className="mobile-connection-options" open={!connectionReady}>
        <summary>
          <span>Connection options</span>
          <small>Access controls, nearby discovery, and advanced URLs</small>
        </summary>
        <div className="mobile-connection-options-body">
          <div className="setting-toggle-row">
            <div>
              <strong>Allow mobile control</strong>
              <span>Let paired phones control threads, runs, Agents, Workers, and approvals.</span>
            </div>
            <Toggle checked={enabled} onChange={updateEnabled} ariaLabel="Allow mobile control" testId="mobile-companion-enabled-toggle" />
          </div>

          <div className="setting-toggle-row">
            <div>
              <strong>Nearby discovery</strong>
              <span>
                Let phones on this trusted network find this desktop
                {lan?.port ? ` on port ${lan.port}` : ""}. Traffic is not encrypted.
              </span>
            </div>
            <Toggle checked={Boolean(lan?.active)} onChange={updateLan} ariaLabel="Enable nearby discovery" testId="mobile-companion-lan-toggle" />
          </div>

          <div className={`mobile-tailscale-status ${tailscaleView.tone}`} data-testid="mobile-tailscale-status">
            <div className="mobile-tailscale-status-icon" aria-hidden="true"><Globe size={18} /></div>
            <div className="mobile-tailscale-status-copy">
              <div className="mobile-tailscale-status-heading">
                <strong>Tailscale remote access</strong>
                <span className="mobile-tailscale-status-badge">{tailscaleView.badge}</span>
              </div>
              <span>{tailscaleView.detail}</span>
              {tailscaleView.endpoint ? <code>{tailscaleView.endpoint}</code> : null}
              <small>Network access only. Pairing is handled separately above.</small>
            </div>
          </div>

          <label className="setting-field">
            <span>Pairing URL fallback</span>
            <input
              data-testid="mobile-companion-url-base"
              value={urlBase}
              onChange={(event) => {
                const next = event.currentTarget.value;
                setUrlBase(next);
                void persistUrlBase(next);
              }}
              placeholder="https://your-pc.your-tailnet.ts.net"
            />
          </label>
          <p className="sheet-hint">
            Used only when remote pairing cannot use a ready Tailscale endpoint.
          </p>
        </div>
      </details>

      <div className="mobile-device-list" data-testid="mobile-companion-device-list">
        <span className="setting-mini-title">Paired devices</span>
        {status?.devices.length ? (
          status.devices.map((device) => (
            <div className="mobile-device-row" key={device.id}>
              <div>
                <strong>{device.name}</strong>
                <span>
                  {device.key_prefix}... · last seen {formatTime(device.last_seen_at)}
                </span>
              </div>
              <button className="btn-ghost danger" type="button" disabled={busy} onClick={() => void revoke(device.id)}>
                Revoke
              </button>
            </div>
          ))
        ) : (
          <p className="sheet-hint">No paired devices yet.</p>
        )}
      </div>
    </div>
  );
}
