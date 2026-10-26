import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** CSV of the caller's active (not archived) projects, sorted by name. Calls back with `(error, { status, body })`. */
export function exportReport(req, callback) {
  db.list("projects", (project) => project.owner === req.user.id && !project.archived, (error, projects) => {
    if (error) return callback(error);
    const rows = projects.sort((a, b) => a.name.localeCompare(b.name)).map((project) => `${project.id},${project.name}`);
    callback(null, { status: 200, body: ["id,name", ...rows].join("\n") });
  });
}
