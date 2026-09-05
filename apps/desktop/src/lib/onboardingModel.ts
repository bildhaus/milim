import type { ModelInfo } from "../api";

export type ConnectedModelSource = { providerId: string } | { owner: string };

export function connectedSourceModel(models: readonly ModelInfo[], source: ConnectedModelSource): ModelInfo | undefined {
  return models.find((model) => "providerId" in source
    ? model.provider_id === source.providerId
    : model.owned_by.toLowerCase() === source.owner.toLowerCase());
}
