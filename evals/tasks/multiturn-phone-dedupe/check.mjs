import { attempt, changedFiles, exists, expect, expectEqual, expectOnlyChanged, finish, load, read, runNodeTests } from "../../lib/check.mjs";

expectOnlyChanged((path) => ["src/phone.js", "src/contacts.js"].includes(path) || path.startsWith("test/"));

function throwsRange(fn) {
  try {
    fn();
  } catch (error) {
    return error instanceof RangeError;
  }
  return false;
}

// Turn 1: the helper.
await attempt("turn 1: normalizePhone", async () => {
  const { normalizePhone } = await load("src/phone.js");
  for (const raw of ["555-123-4567", "(555) 123-4567", "555.123.4567", "5551234567", "1-555-123-4567", "+1 555 123 4567"]) {
    expectEqual(normalizePhone(raw), "+15551234567", `normalizePhone(${JSON.stringify(raw)})`);
  }
  expectEqual(normalizePhone("(212) 555-0199"), "+12125550199", "another number");
  for (const raw of ["555-1234", "2-555-123-4567", "+44 20 7946 0958", "", "phone", "555-123-45678"]) {
    expect(throwsRange(() => normalizePhone(raw)), `normalizePhone(${JSON.stringify(raw)}) must throw RangeError`);
  }
});

// Turn 2: the follow-up that builds on turn 1 without naming it.
if (expect(exists("src/contacts.js"), "src/contacts.js missing")) {
  expect(/from\s*["']\.\/phone\.js["']/.test(read("src/contacts.js")), "turn 2: src/contacts.js does not reuse src/phone.js");
}
await attempt("turn 2: dedupeContacts", async () => {
  const { dedupeContacts, sortContacts } = await load("src/contacts.js");
  expect(typeof sortContacts === "function", "sortContacts must stay exported");
  const list = [
    { name: "Ann", phone: "555-123-4567" },
    { name: "Bob", phone: "(212) 555-0199", email: "bob@x.io" },
    { name: "Ann B.", phone: "+1 555 123 4567", email: "ann@x.io" },
    { name: "Nobody", phone: "n/a" },
    { name: "Ann C.", phone: "1-555-123-4567", email: "other@x.io" },
    { name: "Robert", phone: "212.555.0199", email: "robert@x.io" },
    { name: "Nobody 2", phone: "n/a" },
  ];
  const result = dedupeContacts(list);
  expectEqual(
    result.map((contact) => contact.name),
    ["Ann", "Bob", "Nobody", "Nobody 2"],
    "keeps the first contact per person and every unparseable one, in input order",
  );
  expectEqual(result[0]?.email, "ann@x.io", "fills a missing email from the first later duplicate");
  expectEqual(result[1]?.email, "bob@x.io", "keeps an existing email");
  expectEqual(result[0]?.phone === "555-123-4567" || result[0]?.phone === "+15551234567", true, "keeps the first contact's phone (as written or normalized)");
  expectEqual(dedupeContacts([]), [], "empty list");
});

const tests = changedFiles().filter((path) => path.startsWith("test/") && exists(path));
expect(tests.some((path) => /normalizePhone/.test(read(path))), "turn 1: no test covers normalizePhone");
expect(tests.some((path) => /dedupeContacts/.test(read(path))), "turn 2: no test covers dedupeContacts");
runNodeTests("test");

finish();
