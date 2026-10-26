// Shared by the solution scripts: copy converted handlers and routers from
// converted/ in the task directory into the repository.
import { copyFileSync } from "node:fs";
import { join } from "node:path";

const source = join(process.env.EVAL_TASK_DIR, "converted");
export const FIRST = ["archiveProject", "createUser", "deleteUser", "exportReport", "getUser", "listProjects"];
export const SECOND = ["renameProject", "resetPassword", "searchUsers", "shareProject", "updateUser", "uploadAvatar"];

export function convert(names) {
  for (const name of names) copyFileSync(join(source, `${name}.js`), `src/handlers/${name}.js`);
}

/** `both` keeps callback support for the migration; `async` awaits only. */
export function router(kind) {
  copyFileSync(join(source, `router.${kind}.js`), "src/router.js");
}

export const leftAnswer = `Converted ${FIRST.join(", ")}; the router now accepts both styles.\nStill to do: ${SECOND.join(", ")}.`;
