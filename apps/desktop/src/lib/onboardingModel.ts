import { isUsableChatModel, type AccountRuntimeKind, type ModelInfo, type ProviderKind } from "../api.js";
import { mediaKindForModelId } from "./media.js";

export type ConnectedModelSource = { providerId: string } | { owner: string };

/** A model that can answer a chat turn, as opposed to an image, video, or music generator. */
export function isChatCapableModel(model: Pick<ModelInfo, "id" | "capabilities">): boolean {
  if (!isUsableChatModel(model.id)) return false;
  const capabilities = model.capabilities;
  if (capabilities?.imageOutput || capabilities?.videoOutput || capabilities?.musicOutput) return false;
  return mediaKindForModelId(model.id) === null;
}

export function connectedSourceModel(models: readonly ModelInfo[], source: ConnectedModelSource): ModelInfo | undefined {
  return models.find((model) => isChatCapableModel(model) && ("providerId" in source
    ? model.provider_id === source.providerId
    : model.owned_by.toLowerCase() === source.owner.toLowerCase()));
}

/** Replicate and fal only generate media, so they cannot back a chat runtime. */
export function isMediaOnlyProviderKind(kind: ProviderKind): boolean {
  return kind === "replicate" || kind === "fal";
}

export function workspaceFolderPlaceholder(platform: string): string {
  if (/(mac|iphone|ipad|darwin)/i.test(platform)) return "/Users/you/project";
  if (/win/i.test(platform)) return "C:\\path\\to\\project";
  return "/home/you/project";
}

export type SignInHelpRuntime = Exclude<AccountRuntimeKind, "codex">;

/** Codex signs in from onboarding; the other CLIs sign in with their own tooling. */
export const ACCOUNT_RUNTIME_SIGN_IN_HELP: Record<SignInHelpRuntime, { instruction: string; url: string }> = {
  claude: {
    instruction: "Run `claude auth login` in a terminal",
    url: "https://docs.milim.ai/models#sign-in-to-an-account-runtime",
  },
  opencode: {
    instruction: "Run `opencode auth login` in a terminal",
    url: "https://docs.milim.ai/models#sign-in-to-an-account-runtime",
  },
  pi: {
    instruction: "Run `pi` in a terminal and use `/login`",
    url: "https://docs.milim.ai/models#sign-in-to-an-account-runtime",
  },
};
