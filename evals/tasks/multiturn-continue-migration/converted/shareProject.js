import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Share a project with `{ userId }`; only its owner may. Sharing twice is a no-op. Resolves to `{ status, body }`. */
export async function shareProject(req) {
  const userId = req.body?.userId;
  const project = await db.promises.find("projects", req.params.id);
  if (!project) throw new HttpError(404, "project not found");
  if (project.owner !== req.user.id) throw new HttpError(403, "only the owner can share a project");
  const user = await db.promises.find("users", userId);
  if (!user) throw new HttpError(400, "unknown userId");
  const sharedWith = project.sharedWith.includes(user.id) ? project.sharedWith : [...project.sharedWith, user.id];
  const updated = await db.promises.update("projects", project.id, { sharedWith });
  return { status: 200, body: updated };
}
