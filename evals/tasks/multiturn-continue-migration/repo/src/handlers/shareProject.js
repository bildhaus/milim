import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Share a project with `{ userId }`; only its owner may. Sharing twice is a no-op. Calls back with `(error, { status, body })`. */
export function shareProject(req, callback) {
  const userId = req.body?.userId;
  db.find("projects", req.params.id, (error, project) => {
    if (error) return callback(error);
    if (!project) return callback(new HttpError(404, "project not found"));
    if (project.owner !== req.user.id) return callback(new HttpError(403, "only the owner can share a project"));
    db.find("users", userId, (userError, user) => {
      if (userError) return callback(userError);
      if (!user) return callback(new HttpError(400, "unknown userId"));
      const sharedWith = project.sharedWith.includes(user.id) ? project.sharedWith : [...project.sharedWith, user.id];
      db.update("projects", project.id, { sharedWith }, (updateError, updated) => {
        if (updateError) return callback(updateError);
        callback(null, { status: 200, body: updated });
      });
    });
  });
}
