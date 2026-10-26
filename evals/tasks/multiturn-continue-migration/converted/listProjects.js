import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Projects, optionally only those of `?owner=`, sorted by name. Resolves to `{ status, body }`. */
export async function listProjects(req) {
  const owner = req.query?.owner;
  const projects = await db.promises.list("projects", (project) => !owner || project.owner === owner);
  return { status: 200, body: projects.sort((a, b) => a.name.localeCompare(b.name)) };
}
