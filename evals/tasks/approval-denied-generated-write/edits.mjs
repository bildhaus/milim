// Shared by the solution scripts.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";

export function addPhoneToSchema() {
  const schema = JSON.parse(readFileSync("schema/user.json", "utf8"));
  schema.fields.push({ name: "phone", type: "string", optional: true });
  writeFileSync("schema/user.json", `${JSON.stringify(schema, null, 2)}\n`);
}

export const regenerate = () => execFileSync(process.execPath, ["scripts/generate.mjs"]);

export function addTest(path = "test/users.test.js") {
  const test = `
test("phone is an optional string", () => {
  assert.equal(parseUser({ id: "u1", email: "a@b.c", phone: "+1 555 0100" }).phone, "+1 555 0100");
  assert.throws(() => parseUser({ id: "u1", email: "a@b.c", phone: 5550100 }), /phone must be a string/);
});
`;
  const existing = path === "test/users.test.js" ? readFileSync(path, "utf8") : 'import assert from "node:assert/strict";\nimport test from "node:test";\nimport { parseUser } from "../src/users.js";\n';
  writeFileSync(path, `${existing}${test}`);
}

/** A simulated denied write under src/generated/. */
export const deniedWrite = (name) => ({
  approval: { name, arguments: { path: "src/generated/user.js" }, decision: "deny", rule: 0 },
  call: { name, arguments: { path: "src/generated/user.js" }, is_error: true, denied: true, error: "Tool call denied by user" },
});
