import type { MobileTailscaleStatus } from "../api";

export type MobileTailscaleTone = "checking" | "ready" | "attention";
export type MobileTailscaleAction = "install" | "configure" | "refresh";

export interface MobileTailscalePresentation {
  tone: MobileTailscaleTone;
  badge: string;
  detail: string;
  endpoint: string | null;
  action: MobileTailscaleAction | null;
  actionLabel: string | null;
}

export function describeMobileTailscale(
  status: MobileTailscaleStatus | null,
  checking: boolean,
  error: string | null,
): MobileTailscalePresentation {
  if (!status) {
    return {
      tone: error ? "attention" : "checking",
      badge: error ? "Status unavailable" : "Checking",
      detail: error
        ? "milim could not check Tailscale. Remote mobile access may be unavailable."
        : "Checking whether this desktop is ready for remote mobile access.",
      endpoint: null,
      action: error ? "refresh" : null,
      actionLabel: error ? "Check again" : null,
    };
  }
  if (!status.installed) {
    return {
      tone: "attention",
      badge: "Not installed",
      detail: "Tailscale is optional on a trusted LAN. Install it to reach this desktop from elsewhere.",
      endpoint: null,
      action: "install",
      actionLabel: "Install Tailscale",
    };
  }
  if (!status.logged_in) {
    return {
      tone: "attention",
      badge: "Not connected",
      detail: "Remote access is offline. Open Tailscale and sign in if needed, then check again.",
      endpoint: null,
      action: "refresh",
      actionLabel: "Check again",
    };
  }
  if (!status.serve_configured) {
    return {
      tone: "attention",
      badge: "Setup needed",
      detail: "Tailscale is connected, but milim remote access is not configured yet.",
      endpoint: status.public_url ?? null,
      action: "configure",
      actionLabel: "Enable remote access",
    };
  }
  return {
    tone: "ready",
    badge: checking ? "Checking" : "Ready",
    detail: status.message ?? "This desktop is reachable from devices on the same tailnet.",
    endpoint: status.public_url ?? null,
    action: null,
    actionLabel: null,
  };
}
