import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** CSV of the caller's active (not archived) projects, sorted by name. Resolves to `{ status, body }`. */
export async function exportReport(req) {
  const projects = await db.promises.list("projects", (project) => project.owner === req.user.id && !project.archived);
  const rows = projects.sort((a, b) => a.name.localeCompare(b.name)).map((project) => `${project.id},${project.name}`);
  return { status: 200, body: ["id,name", ...rows].join("\n") };
}
