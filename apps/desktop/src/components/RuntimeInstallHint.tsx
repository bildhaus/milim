import { useState } from "react";
import { setAccountRuntimeBinary, type AccountRuntimeKind } from "../api";
import {
  RUNTIME_CLI_LABELS,
  RUNTIME_INSTALL_COMMANDS,
} from "../lib/runtimeInstall";
import { Check, Copy } from "./icons";
import "./ProvidersManager.css";

/** Ask for an executable with the native file picker and save it as the
 * runtime's binary override. Resolves false when the picker is cancelled. */
async function locateRuntimeBinary(runtime: AccountRuntimeKind): Promise<boolean> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selected = await open({
    multiple: false,
    directory: false,
    title: `Locate the ${RUNTIME_CLI_LABELS[runtime]} executable`,
  });
  if (typeof selected !== "string" || !selected) return false;
  await setAccountRuntimeBinary(runtime, selected);
  return true;
}

/**
 * Install guidance and the "Locate binary..." override for one account
 * runtime. When the CLI is missing it shows a copyable install command;
 * otherwise it stays a compact link, plus the active override with Reset.
 */
export function RuntimeInstallHint({
  runtime,
  missing,
  overridePath,
  onChanged,
}: {
  runtime: AccountRuntimeKind;
  missing: boolean;
  overridePath?: string;
  onChanged?: () => void;
}) {
  const [copied, setCopied] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const label = RUNTIME_CLI_LABELS[runtime];
  const command = RUNTIME_INSTALL_COMMANDS[runtime];

  async function change(action: () => Promise<boolean>) {
    setBusy(true);
    setError("");
    try {
      if (await action()) onChanged?.();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  }

  const locate = (
    <button
      className="btn-ghost"
      type="button"
      data-testid={`${runtime}-locate-binary`}
      disabled={busy}
      onClick={() => void change(() => locateRuntimeBinary(runtime))}
    >
      {overridePath ? "Change..." : "Locate binary..."}
    </button>
  );

  return (
    <div className={missing ? "account-profile-hint runtime-install-hint" : "runtime-binary-line"}>
      {missing && (
        <>
          <p>
            {label} CLI not found. Install it with the command below, or locate
            an existing executable (for example one managed by nvm or fnm).
          </p>
          <div className="account-profile-command">
            <code>{command}</code>
            <button
              className="btn-ghost"
              type="button"
              title="Copy the install command"
              aria-label={`Copy the ${label} install command`}
              onClick={() => {
                void navigator.clipboard.writeText(command).then(() => {
                  setCopied(true);
                  setTimeout(() => setCopied(false), 1500);
                });
              }}
            >
              {copied ? <Check size={13} /> : <Copy size={13} />}
            </button>
          </div>
        </>
      )}
      <div className="runtime-binary-actions">
        {overridePath && (
          <span className="runtime-binary-path" title={overridePath}>
            Using <code>{overridePath}</code>
          </span>
        )}
        {locate}
        {overridePath && (
          <button
            className="btn-ghost"
            type="button"
            disabled={busy}
            title="Find the CLI automatically again"
            onClick={() =>
              void change(async () => {
                await setAccountRuntimeBinary(runtime, null);
                return true;
              })
            }
          >
            Reset
          </button>
        )}
      </div>
      {error && <p className="runtime-binary-error" role="alert">{error}</p>}
    </div>
  );
}
