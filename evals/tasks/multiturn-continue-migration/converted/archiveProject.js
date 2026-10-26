import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Archive a project; only its owner may. Resolves to `{ status, body }`. */
export async function archiveProject(req) {
  const project = await db.promises.find("projects", req.params.id);
  if (!project) throw new HttpError(404, "project not found");
  if (project.owner !== req.user.id) throw new HttpError(403, "only the owner can archive a project");
  const updated = await db.promises.update("projects", project.id, { archived: true });
  return { status: 200, body: updated };
}
