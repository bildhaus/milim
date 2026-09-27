import { attempt, expect, expectEqual, expectOnlyChanged, finish, headFile, lines, load, read } from "../../lib/check.mjs";

// `retry: { attempts: 3, backoffMs: 250 },` appears ten times, and the
// payments block is textually identical in both environments apart from
// its base URL. Only production.payments may change.
const PATH = "config/clients.js";
expectOnlyChanged([PATH]);

/** [start, end] line indexes of the production payments block. */
function span(all) {
  const production = all.indexOf("  production: {");
  const start = all.findIndex((line, index) => index > production && line === "    payments: {");
  const end = all.findIndex((line, index) => index > start && line === "    },");
  return production < 0 || start < 0 || end < 0 ? null : [start, end];
}

/** 1-based line numbers where `a` and `b` differ, compared from the start. */
function differing(a, b) {
  const out = [];
  for (let index = 0; index < Math.max(a.length, b.length); index += 1) if (a[index] !== b[index]) out.push(index + 1);
  return out;
}

const before = lines(headFile(PATH));
const after = lines(read(PATH));
const old = span(before);
const now = span(after);
if (expect(now !== null, "production payments block not found")) {
  const above = differing(after.slice(0, now[0]), before.slice(0, old[0]));
  expect(above.length === 0, `lines changed above production.payments: ${above.slice(0, 5).join(", ")}`);
  const below = differing(after.slice(now[1] + 1), before.slice(old[1] + 1)).map((line) => line + now[1] + 1);
  expect(below.length === 0, `lines changed below production.payments: ${below.slice(0, 5).join(", ")}`);
}

await attempt("settings", async () => {
  const { CLIENTS } = await load(PATH);
  const { CLIENTS: ORIGINAL } = await import(`data:text/javascript,${encodeURIComponent(headFile(PATH))}`);
  const expected = structuredClone(ORIGINAL);
  expected.production.payments.retry.attempts = 5;
  expectEqual(CLIENTS.production?.payments, expected.production.payments, "production.payments");
  const changed = Object.entries(expected).flatMap(([env, clients]) =>
    Object.entries(clients)
      .filter(([name, settings]) => JSON.stringify(CLIENTS[env]?.[name]) !== JSON.stringify(settings))
      .map(([name]) => `${env}.${name}`),
  );
  expect(changed.every((path) => path === "production.payments"), `other clients changed: ${changed.filter((path) => path !== "production.payments").join(", ")}`);
  expectEqual(Object.keys(CLIENTS), Object.keys(expected), "environments");
});

finish();
