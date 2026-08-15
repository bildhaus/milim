import { useEffect, useMemo, useState } from "react";
import QRCode from "qrcode";
import {
  apiBaseUrl,
  configureMobileTailscaleRelay,
  getControlBootstrap,
  getMobileCompanionStatus,
  getMobileLanStatus,
  openExternalUrl,
  revokeMobileCompanionDevice,
  setMobileCompanionEnabled,
  setMobileLanEnabled,
  startMobileCompanionPairing,
  type MobileCompanionStatus,
  type MobileLanStatus,
} from "../api";
import { readUserStateKey, writeUserStateKey } from "../persistence/userStateStorage";
import { Copy, Refresh } from "./icons";
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
  const [tailscaleInstallVisible, setTailscaleInstallVisible] = useState(false);
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

  const pairingUrl = useMemo(() => {
    if (!status?.pairing || !urlBase.trim()) return "";
    return `${normalizeBase(urlBase)}${status.pairing.path}`;
  }, [status?.pairing, urlBase]);

  const nativePairingUrl = useMemo(() => {
    if (!status?.pairing || !pairingUrl) return "";
    const query = new URLSearchParams({
      endpoint: normalizeBase(urlBase),
      pair_id: status.pairing.id,
      secret: new URL(pairingUrl).searchParams.get("secret") ?? "",
      host_id: hostId || lan?.host_id || "",
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
    try {
      const [nextStatus, nextLan] = await Promise.all([
        getMobileCompanionStatus(),
        getMobileLanStatus(),
      ]);
      setStatus(nextStatus);
      setLan(nextLan);
      setNotice({ tone: "success", message: "Mobile companion status refreshed." });
    } catch (error) {
      setNotice({ tone: "error", message: `Refresh failed: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
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

  async function createPairing(baseValue: string, message = "Pairing link created. Open it on the phone.") {
    const pairing = await startMobileCompanionPairing();
    setStatus((current) =>
      current
        ? { ...current, enabled: true, pairing }
        : { enabled: true, pairing, devices: [], queued_events: 0 },
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
      await createPairing(urlBase);
    } catch (error) {
      setNotice({ tone: "error", message: `Pairing failed: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setBusy(false);
    }
  }

  async function setupTailscale() {
    setBusy(true);
    setTailscaleBusy(true);
    setNotice({ tone: "info", message: "Setting up Tailscale Serve..." });
    try {
      if (!enabled) {
        setStatus(await setMobileCompanionEnabled(true));
      }
      const relay = await configureMobileTailscaleRelay();
      if (!relay.installed) {
        setTailscaleInstallVisible(true);
        await openTailscaleDownload("Tailscale is not installed. I opened the official download page. Install it, sign in, then click Set up with Tailscale again.");
        return;
      }
      setTailscaleInstallVisible(false);
      if (!relay.logged_in || !relay.public_url) {
        throw new Error(relay.message || "Tailscale is not ready.");
      }
      if (!relay.serve_configured) {
        throw new Error(relay.message || "Tailscale Serve did not report the mobile relay target.");
      }
      await createPairing(relay.public_url, relay.message || "Tailscale Serve is ready. Open the pairing link on your phone.");
    } catch (error) {
      setNotice({ tone: "error", message: `Tailscale setup failed: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setTailscaleBusy(false);
      setBusy(false);
    }
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

  return (
    <div className="mobile-companion-settings">
      <div className="setting-toggle-row">
        <div>
          <strong>Enable companion bridge</strong>
          <span>Allow paired native and web clients to control threads, runs, Agents, Workers, and approvals.</span>
        </div>
        <Toggle checked={enabled} onChange={updateEnabled} ariaLabel="Enable companion bridge" testId="mobile-companion-enabled-toggle" />
      </div>

      <div className="setting-toggle-row">
        <div>
          <strong>Trusted-network LAN discovery</strong>
          <span>
            Advertise <code>_milim._tcp.local.</code>
            {lan?.port ? ` on port ${lan.port}` : ""}. Disabled by default; traffic is not encrypted.
          </span>
        </div>
        <Toggle checked={Boolean(lan?.active)} onChange={updateLan} ariaLabel="Enable trusted-network LAN discovery" testId="mobile-companion-lan-toggle" />
      </div>

      <label className="setting-field">
        <span>Phone URL base</span>
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
        Use a Tailscale Serve URL on a real phone, or <code>http://10.0.2.2:&lt;port&gt;</code> in the Android emulator.
      </p>

      <div className="mobile-companion-actions">
        <button className="btn-ghost" type="button" disabled={busy} onClick={() => void setupTailscale()} data-testid="mobile-companion-tailscale-setup">
          {tailscaleBusy ? "Setting up..." : "Set up with Tailscale"}
        </button>
        {tailscaleInstallVisible && (
          <button className="btn-ghost" type="button" disabled={busy} onClick={() => void openTailscaleDownload()} data-testid="mobile-companion-tailscale-install">
            Install Tailscale
          </button>
        )}
        <button className="btn-ghost" type="button" disabled={!enabled || busy} onClick={() => void startPairing()} data-testid="mobile-companion-start-pairing">
          Pair device
        </button>
        <button className="btn-ghost" type="button" disabled={busy} onClick={() => void refresh()} title="Refresh mobile companion">
          <Refresh size={13} /> Refresh
        </button>
      </div>

      {pairingUrl && (
        <div className="mobile-pairing-panel">
          {qrDataUrl && <img className="mobile-pairing-qr" src={qrDataUrl} alt="Mobile companion pairing QR code" />}
          <div className="mobile-pairing-copy">
            <span className="setting-mini-title">Native pairing link</span>
            <code data-testid="mobile-companion-pairing-url">{nativePairingUrl}</code>
            <button className="btn-ghost" type="button" onClick={() => void copyPairingUrl()}>
              <Copy size={13} /> Copy native link
            </button>
            <small>Legacy web companion: {pairingUrl}</small>
            <small>Expires {formatTime(status?.pairing?.expires_at)}.</small>
          </div>
        </div>
      )}

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

      {notice && (
        <p className={`mobile-companion-notice ${notice.tone}`} role={notice.tone === "error" ? "alert" : "status"}>
          {notice.message}
        </p>
      )}
    </div>
  );
}
