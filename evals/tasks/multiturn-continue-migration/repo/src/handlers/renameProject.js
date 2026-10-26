import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Rename a project; only its owner may. Calls back with `(error, { status, body })`. */
export function renameProject(req, callback) {
  const name = req.body?.name;
  if (typeof name !== "string" || !name.trim()) return callback(new HttpError(400, "name is required"));
  db.find("projects", req.params.id, (error, project) => {
    if (error) return callback(error);
    if (!project) return callback(new HttpError(404, "project not found"));
    if (project.owner !== req.user.id) return callback(new HttpError(403, "only the owner can rename a project"));
    db.update("projects", project.id, { name: name.trim() }, (updateError, updated) => {
      if (updateError) return callback(updateError);
      callback(null, { status: 200, body: updated });
    });
  });
}
