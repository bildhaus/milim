import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Rename a project; only its owner may. Resolves to `{ status, body }`. */
export async function renameProject(req) {
  const name = req.body?.name;
  if (typeof name !== "string" || !name.trim()) throw new HttpError(400, "name is required");
  const project = await db.promises.find("projects", req.params.id);
  if (!project) throw new HttpError(404, "project not found");
  if (project.owner !== req.user.id) throw new HttpError(403, "only the owner can rename a project");
  const updated = await db.promises.update("projects", project.id, { name: name.trim() });
  return { status: 200, body: updated };
}
