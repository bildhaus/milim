import { equal } from "node:assert/strict";
import { connectedSourceModel } from "../src/lib/onboardingModel.js";

const old = { id: "old", owned_by: "OpenAI", provider_id: "old-provider" };
const connected = { id: "new", owned_by: "OpenAI", provider_id: "new-provider" };
equal(connectedSourceModel([old], { providerId: "new-provider" }), undefined, "a cached unrelated source must not be selected while refresh is pending");
equal(connectedSourceModel([old, connected], { providerId: "new-provider" }), connected, "same-brand providers must still be selected by exact provider id");
equal(connectedSourceModel([old], { owner: "Codex" }), undefined, "an unavailable account runtime must not fall back to another source");
const codex = { id: "codex:model", owned_by: "Codex" };
equal(connectedSourceModel([old, codex], { owner: "codex" }), codex);
