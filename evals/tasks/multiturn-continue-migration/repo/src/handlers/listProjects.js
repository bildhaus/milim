import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Projects, optionally only those of `?owner=`, sorted by name. Calls back with `(error, { status, body })`. */
export function listProjects(req, callback) {
  const owner = req.query?.owner;
  db.list("projects", (project) => !owner || project.owner === owner, (error, projects) => {
    if (error) return callback(error);
    callback(null, { status: 200, body: projects.sort((a, b) => a.name.localeCompare(b.name)) });
  });
}
