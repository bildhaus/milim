import { play, setEnabled, type SoundName } from "cuelume";
import type { ChatMessage } from "../api";

const DISABLED_SELECTOR = ":disabled, [aria-disabled='true']";
const SILENT_SELECTOR = ".win-btn, .ui-slider";
const SOUND_TARGETS: ReadonlyArray<readonly [string, SoundName]> = [
  ["[data-interface-sound='droplet'], .send-btn.stop, .sheet-close, [aria-label^='Close ']", "droplet"],
  ["[role='switch'], [role='checkbox'], [role='radio'], [role='tab']", "toggle"],
  ["[role='menuitem'], .ui-select-item", "tick"],
  [".send-btn, .btn-accent", "press"],
];

setEnabled(false);

export function interfaceSoundForTarget(target: Pick<Element, "closest">): SoundName | null {
  if (target.closest(SILENT_SELECTOR)) return null;
  for (const [selector, sound] of SOUND_TARGETS) {
    const matched = target.closest(selector);
    if (!matched) continue;
    return matched.matches(DISABLED_SELECTOR) ? null : sound;
  }
  return null;
}

export function pendingAttentionKey(
  messages: ChatMessage[],
  proposedWorkerRunId?: string,
): string | null {
  if (proposedWorkerRunId) return `worker:${proposedWorkerRunId}`;
  const seenStreamApprovalIds = new Set<string>();
  for (let messageIndex = messages.length - 1; messageIndex >= 0; messageIndex -= 1) {
    const message = messages[messageIndex];
    const messageKey = message.id ?? String(messageIndex);
    if (message.approval?.status === "pending") {
      return `message:${messageKey}:${message.approval.kind}`;
    }
    const pendingStep = message.run?.steps.find((step) => step.approval?.status === "pending");
    if (pendingStep?.approval) return `tool:${messageKey}:${pendingStep.approval.id}`;
    const streamParts = message.streamParts ?? [];
    for (let partIndex = streamParts.length - 1; partIndex >= 0; partIndex -= 1) {
      const part = streamParts[partIndex];
      if (part.kind !== "event" || !part.approvalStatus) continue;
      if (part.approvalId) {
        if (seenStreamApprovalIds.has(part.approvalId)) continue;
        seenStreamApprovalIds.add(part.approvalId);
      }
      if (part.approvalStatus === "pending") {
        return `tool:${messageKey}:${part.approvalId ?? part.callId ?? part.label}`;
      }
    }
  }
  return null;
}

export function installInterfaceSoundClicks(root: Document = document): () => void {
  const onClick = (event: MouseEvent) => {
    if (event.defaultPrevented || !(event.target instanceof Element)) return;
    const sound = interfaceSoundForTarget(event.target);
    if (sound) play(sound);
  };
  root.addEventListener("click", onClick);
  return () => root.removeEventListener("click", onClick);
}

export function setInterfaceSoundsEnabled(enabled: boolean): void {
  setEnabled(enabled);
}

export function playInterfaceSound(sound: SoundName): void {
  play(sound);
}
