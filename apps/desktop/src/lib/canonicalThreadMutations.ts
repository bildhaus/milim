import {
  createControlCommandId,
  sendControlCommand,
} from "../api.js";
import { flushDeferredUserStateWrites } from "../persistence/userStateStorage.js";

async function syncThreadMutation(
  sessionId: string,
  kind:
    | "thread.set_agent"
    | "thread.set_execution_settings"
    | "thread.set_account_profile",
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

/**
 * Which signed-in account of one runtime a thread uses. `null` returns the
 * thread to the runtime's own configuration home; `auto` re-picks per turn.
 */
export function syncCanonicalAccountProfile(
  sessionId: string,
  runtime: string,
  profileId: string | null,
): Promise<void> {
  return syncThreadMutation(
    sessionId,
    "thread.set_account_profile",
    { runtime, profile_id: profileId },
    "Account",
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
