import {
  createControlCommandId,
  sendControlCommand,
} from "../api.js";
import { flushDeferredUserStateWrites } from "../persistence/userStateStorage.js";

async function syncThreadMutation(
  sessionId: string,
  kind: "thread.set_agent" | "thread.set_execution_settings",
  payload: Record<string, string | boolean | null>,
  label: string,
): Promise<void> {
  await flushDeferredUserStateWrites("milim.sessions");
  const result = await sendControlCommand({
    command_id: createControlCommandId(),
    kind,
    thread_id: sessionId,
    payload,
  });
  if (result.status !== "applied") {
    throw new Error(result.message || `${label} synchronization ${result.status}.`);
  }
}

export function syncCanonicalExecutionSettings(
  sessionId: string,
  payload: Record<string, string | boolean | null>,
): Promise<void> {
  return syncThreadMutation(
    sessionId,
    "thread.set_execution_settings",
    payload,
    "Execution setting",
  );
}

export function syncCanonicalThreadAgent(
  sessionId: string,
  agentId: string | null,
): Promise<void> {
  return syncThreadMutation(
    sessionId,
    "thread.set_agent",
    { agent_id: agentId },
    "Agent",
  );
}
