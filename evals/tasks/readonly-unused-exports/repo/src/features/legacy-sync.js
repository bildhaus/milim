// Old implementation, superseded by sync.js. Nothing imports this file.
import { kebab } from "../lib/strings.js";

export function legacySync(name) {
  return `legacy-${kebab(name)}`;
}
