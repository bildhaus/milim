import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Archive a project; only its owner may. Calls back with `(error, { status, body })`. */
export function archiveProject(req, callback) {
  db.find("projects", req.params.id, (error, project) => {
    if (error) return callback(error);
    if (!project) return callback(new HttpError(404, "project not found"));
    if (project.owner !== req.user.id) return callback(new HttpError(403, "only the owner can archive a project"));
    db.update("projects", project.id, { archived: true }, (updateError, updated) => {
      if (updateError) return callback(updateError);
      callback(null, { status: 200, body: updated });
    });
  });
}
