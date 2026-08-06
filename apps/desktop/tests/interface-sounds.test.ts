const { interfaceSoundForTarget, pendingAttentionKey } = await import("../src/ui/sounds.js");
const {
  nativeBadgeThreadCount,
  setMilimUnreadBadge,
  unreadBadgeLabel,
} = await import("../src/lib/nativeNotifications.js");
const { matchingSettingsEntries } = await import("../src/settings/search.js");

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

function target(matches: string[], disabled = false): Parameters<typeof interfaceSoundForTarget>[0] {
  const element = {
    matches: (selector: string) => disabled && selector.includes(":disabled"),
  } as Element;
  return {
    closest: (selector: string) => matches.some((match) => selector.includes(match)) ? element : null,
  } as Parameters<typeof interfaceSoundForTarget>[0];
}

equal(interfaceSoundForTarget(target([".send-btn.stop", ".send-btn"])), "droplet", "stop should beat primary action sound");
equal(interfaceSoundForTarget(target(["[role='switch']"])), "toggle", "switches should use toggle");
equal(interfaceSoundForTarget(target(["[role='tab']"])), "toggle", "tabs should use toggle");
equal(interfaceSoundForTarget(target([".ui-select-item"])), "tick", "select options should use tick");
equal(interfaceSoundForTarget(target([".btn-accent"])), "press", "primary actions should use press");
equal(interfaceSoundForTarget(target([".btn-accent"], true)), null, "disabled controls should stay silent");
equal(interfaceSoundForTarget(target([".win-btn", ".btn-accent"])), null, "window controls should stay silent");
equal(interfaceSoundForTarget(target([".btn-ghost"])), null, "routine buttons should stay silent");
equal(pendingAttentionKey([], "worker-1"), "worker:worker-1", "proposed worker plans should request attention");
equal(pendingAttentionKey([{
  id: "message-1",
  role: "assistant",
  content: "",
  approval: { kind: "tool", scope: "reply", status: "pending", requestedAt: 1 },
}]), "message:message-1:tool", "pending message approvals should request attention");
equal(pendingAttentionKey([{
  id: "message-2",
  role: "assistant",
  content: "",
  streamParts: [{ kind: "event", eventType: "tool", label: "Approve", approvalId: "approval-2", approvalStatus: "pending" }],
}]), "tool:message-2:approval-2", "pending streamed tool approvals should request attention");
equal(pendingAttentionKey([{ role: "assistant", content: "done" }]), null, "ordinary messages should not request attention");
const pendingApproval = {
  id: "approval-message",
  role: "assistant",
  content: "",
  streamParts: [{ kind: "event" as const, eventType: "tool" as const, label: "Approve", approvalId: "approval-1", approvalStatus: "pending" as const }],
};
equal(nativeBadgeThreadCount({ sessions: [{ id: "unread", messages: [] }], unreadSessionIds: ["unread"], workerRuns: [] }), 1, "unread threads should count toward the native badge");
equal(nativeBadgeThreadCount({ sessions: [{ id: "approval", messages: [pendingApproval] }], unreadSessionIds: [], workerRuns: [] }), 1, "pending approvals should count toward the native badge");
equal(nativeBadgeThreadCount({ sessions: [{ id: "approval", messages: [pendingApproval] }], unreadSessionIds: ["approval"], workerRuns: [] }), 1, "unread attention threads should count only once");
equal(nativeBadgeThreadCount({ sessions: [{ id: "approval", messages: [{ ...pendingApproval, streamParts: [{ ...pendingApproval.streamParts[0], approvalStatus: "approved" }] }] }], unreadSessionIds: [], workerRuns: [] }), 0, "resolved approvals should stop counting");
equal(nativeBadgeThreadCount({ sessions: [{ id: "worker", messages: [] }], unreadSessionIds: [], workerRuns: [{ run: { id: "run-1", parent_thread_id: "worker", status: "proposed" } }] }), 1, "proposed Worker Runs should count toward the native badge");
equal(nativeBadgeThreadCount({ sessions: [], unreadSessionIds: [], workerRuns: [] }), 0, "empty state should clear the native badge");
equal(unreadBadgeLabel(0), null, "zero unread threads should have no badge label");
equal(unreadBadgeLabel(9), "9", "single-digit badge counts should stay exact");
equal(unreadBadgeLabel(10), "9+", "large Windows badge counts should clamp to 9+");
await setMilimUnreadBadge(1);
equal(matchingSettingsEntries("sound")[0]?.id, "appearance-interface-sounds", "sound search should find the setting");
equal(matchingSettingsEntries("audio")[0]?.id, "appearance-interface-sounds", "audio search should find the setting");
equal(matchingSettingsEntries("attention")[0]?.id, "appearance-interface-sounds", "attention search should find the setting");

export {};
