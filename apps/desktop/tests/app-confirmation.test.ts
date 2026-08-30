import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import {
  confirmApp,
  resolveAppConfirmation,
  useAppConfirmation,
} from "../src/ui/confirmation.js";

const accepted = confirmApp({
  title: "Apply changes?",
  message: "Apply this change now?",
  confirmLabel: "Apply",
});
assert.equal(useAppConfirmation.getState().request?.title, "Apply changes?");
resolveAppConfirmation(true);
assert.equal(await accepted, true);
assert.equal(useAppConfirmation.getState().request, null);

const superseded = confirmApp({ title: "First", message: "First request" });
const active = confirmApp({ title: "Second", message: "Second request" });
assert.equal(await superseded, false, "a newer confirmation should safely cancel the older request");
resolveAppConfirmation(false);
assert.equal(await active, false);

for (const file of [
  "src/components/ChatView.tsx",
  "src/components/GoogleWorkspacePreview.tsx",
  "src/components/WorkersInspector.tsx",
  "src/lib/newChatCoordinator.ts",
  "src/settings/SettingsDialog.tsx",
]) {
  assert.equal(
    readFileSync(resolve(process.cwd(), file), "utf8").includes("window.confirm"),
    false,
    `${file} should use the app confirmation surface`,
  );
}
