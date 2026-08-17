import { readFileSync } from "node:fs";
import { describeMobileTailscale } from "../src/components/mobileTailscaleStatus.js";

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

const base = {
  installed: true,
  logged_in: true,
  serve_configured: true,
  public_url: "https://desktop.example.ts.net:10000",
  local_target: "http://127.0.0.1:7378",
  message: null,
};

equal(describeMobileTailscale(null, true, null).badge, "Checking", "initial status should be explicit");
equal(describeMobileTailscale(null, false, "timeout").badge, "Status unavailable", "status failures should be visible");
equal(
  describeMobileTailscale({ ...base, installed: false, logged_in: false, serve_configured: false }, false, null).action,
  "install",
  "missing Tailscale should offer installation",
);
equal(
  describeMobileTailscale({ ...base, logged_in: false, serve_configured: false, public_url: null }, false, null).badge,
  "Not connected",
  "inactive Tailscale should not look ready",
);
equal(
  describeMobileTailscale({ ...base, serve_configured: false }, false, null).action,
  "configure",
  "connected Tailscale without Serve should offer setup",
);
equal(
  describeMobileTailscale({ ...base, serve_configured: false }, false, null).actionLabel,
  "Enable remote access",
  "Tailscale should be described as transport setup rather than pairing",
);
equal(describeMobileTailscale(base, false, null).badge, "Ready", "configured Tailscale should report ready");
equal(describeMobileTailscale(base, false, null).action, null, "healthy Tailscale should not show a redundant refresh action");
equal(
  describeMobileTailscale(base, false, null).endpoint,
  base.public_url,
  "ready status should expose the reachable endpoint",
);

const settingsSource = readFileSync("src/components/MobileCompanionSettings.tsx", "utf8");
const configureStart = settingsSource.indexOf("async function configureTailscaleRemoteAccess");
const configureEnd = settingsSource.indexOf("async function runTailscaleAction", configureStart);
const configureSource = settingsSource.slice(configureStart, configureEnd);
equal(configureStart >= 0 && configureEnd > configureStart, true, "remote access setup should remain explicit");
equal(configureSource.includes("createPairing("), false, "remote access setup must not create a pairing link");
equal(
  settingsSource.includes("mobile-companion-tailscale-setup"),
  false,
  "the old Tailscale pairing button should stay removed",
);
equal(settingsSource.includes("Enable companion bridge"), false, "internal bridge terminology should stay out of the setup flow");
equal(settingsSource.includes("Show QR or link"), false, "pairing should use one clear code action");
equal(settingsSource.includes("mobile-connection-options"), true, "advanced connection controls should stay collapsible");

export {};
