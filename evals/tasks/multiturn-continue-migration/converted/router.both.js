import { archiveProject } from "./handlers/archiveProject.js";
import { createUser } from "./handlers/createUser.js";
import { deleteUser } from "./handlers/deleteUser.js";
import { exportReport } from "./handlers/exportReport.js";
import { getUser } from "./handlers/getUser.js";
import { listProjects } from "./handlers/listProjects.js";
import { renameProject } from "./handlers/renameProject.js";
import { resetPassword } from "./handlers/resetPassword.js";
import { searchUsers } from "./handlers/searchUsers.js";
import { shareProject } from "./handlers/shareProject.js";
import { updateUser } from "./handlers/updateUser.js";
import { uploadAvatar } from "./handlers/uploadAvatar.js";
import { HttpError } from "./errors.js";

const routes = [
  ["POST", "/projects/:id/archive", archiveProject],
  ["POST", "/users", createUser],
  ["DELETE", "/users/:id", deleteUser],
  ["GET", "/reports/projects.csv", exportReport],
  ["GET", "/users/:id", getUser],
  ["GET", "/projects", listProjects],
  ["PATCH", "/projects/:id", renameProject],
  ["POST", "/password-resets", resetPassword],
  ["GET", "/users", searchUsers],
  ["POST", "/projects/:id/shares", shareProject],
  ["PATCH", "/users/:id", updateUser],
  ["PUT", "/users/:id/avatar", uploadAvatar],
];

function match(pattern, path) {
  const want = pattern.split("/");
  const got = path.split("/");
  if (want.length !== got.length) return null;
  const params = {};
  for (const [index, part] of want.entries()) {
    if (part.startsWith(":")) params[part.slice(1)] = decodeURIComponent(got[index]);
    else if (part !== got[index]) return null;
  }
  return params;
}

/**
 * Route `method path` to its handler. Resolves to `{ status, body }`; an
 * HttpError becomes `{ status, body: { error } }`.
 */
export async function dispatch(method, path, req = {}) {
  const [pathname, search = ""] = path.split("?");
  for (const [routeMethod, pattern, handler] of routes) {
    const params = routeMethod === method ? match(pattern, pathname) : null;
    if (!params) continue;
    const request = { user: { id: "u1" }, body: {}, ...req, params, query: Object.fromEntries(new URLSearchParams(search)) };
    try {
      // Callback handlers are still being migrated to async functions.
      if (handler.length >= 2) {
        return await new Promise((resolve, reject) => {
          handler(request, (error, response) => (error ? reject(error) : resolve(response)));
        });
      }
      return await handler(request);
    } catch (error) {
      if (error instanceof HttpError) return { status: error.status, body: { error: error.message } };
      throw error;
    }
  }
  return { status: 404, body: { error: "no route" } };
}
