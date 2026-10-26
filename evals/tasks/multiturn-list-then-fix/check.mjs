import {
  expect,
  expectOnlyChanged,
  fail,
  finish,
  headFile,
  loadSource,
  read,
  requireTrajectory,
  turnAnswer,
  turnChangedFiles,
  turnFile,
} from "../../lib/check.mjs";

// Three independent failures, one per module. Which one is "second" and
// which "first" depends only on the order the agent itself listed them in
// turn 1, so the check reads that answer and follows it.
const FAILURES = {
  cart: { file: "src/cart.js", aliases: ["half-cent", "carttotal", "cart.test", "cart total rounds", "src/cart"] },
  dates: { file: "src/dates.js", aliases: ["isoweek(", "isoweek ", "isoweek`", "iso week", "iso year", "early january", "dates.test", "src/dates"] },
  sku: { file: "src/sku.js", aliases: ["findduplicateskus", "duplicate sku", "sku.test", "src/sku"] },
};

/** Whether a module's source fixes its failure (hidden cases, not the repository's tests). */
const FIXED = {
  async cart(source) {
    const { cartTotal, formatCents } = await loadSource(source, "cart.mjs");
    return (
      cartTotal([{ cents: 105, qty: 1 }], 0.5) === 53 &&
      cartTotal([{ cents: 1001, qty: 1 }], 0.5) === 501 &&
      cartTotal([{ cents: 1000, qty: 3 }], 0.25) === 2250 &&
      cartTotal([{ cents: 250, qty: 2 }, { cents: 99, qty: 1 }]) === 599 &&
      formatCents(5) === "$0.05"
    );
  },
  async dates(source) {
    const { isoWeek, isoWeekday } = await loadSource(source, "dates.mjs");
    const cases = { "2021-01-03": 53, "2022-01-02": 52, "2024-06-12": 24, "2024-12-30": 1, "2020-12-31": 53, "2023-01-01": 52, "2026-01-01": 1, "2027-01-01": 53, "2025-12-29": 1 };
    return Object.entries(cases).every(([date, week]) => isoWeek(date) === week) && isoWeekday("2024-03-10") === 7;
  },
  async sku(source) {
    const { findDuplicateSkus } = await loadSource(source, "sku.mjs");
    const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
    return (
      same(findDuplicateSkus([{ sku: "ABC-0001" }, { sku: "abc-0001 " }, { sku: "XYZ-0002" }, { sku: "Abc-0001" }]), ["ABC-0001"]) &&
      same(findDuplicateSkus([{ sku: " xyz-0002" }, { sku: "XYZ-0002" }, { sku: "abc-0001" }, { sku: "ABC-0001" }]), ["XYZ-0002", "ABC-0001"]) &&
      same(findDuplicateSkus([{ sku: "ABC-0001" }, { sku: "ABC-0002" }]), [])
    );
  },
};

async function isFixed(key, source) {
  if (source === null) return false;
  try {
    return await FIXED[key](source);
  } catch {
    return false;
  }
}

for (const path of ["test/cart.test.js", "test/dates.test.js", "test/sku.test.js", "test/names.test.js", "src/names.js"]) {
  expect(read(path) === headFile(path), `${path} was modified`);
}
expectOnlyChanged(Object.values(FAILURES).map((failure) => failure.file));

const trajectory = requireTrajectory();
if (trajectory) {
  expect(trajectory.turns.length === 3, `expected 3 recorded turns, got ${trajectory.turns.length}`);
  const turnOne = turnChangedFiles(1) ?? [];
  expect(turnOne.length === 0, `turn 1 was asked not to change files but changed: ${turnOne.join(", ")}`);

  // The order of first mention of each failure in the turn-1 answer.
  const answer = turnAnswer(1).toLowerCase();
  const positions = Object.entries(FAILURES).map(([key, failure]) => {
    const found = failure.aliases.map((alias) => answer.indexOf(alias)).filter((index) => index >= 0);
    return { key, at: found.length ? Math.min(...found) : -1 };
  });
  const missing = positions.filter((entry) => entry.at < 0).map((entry) => entry.key);
  if (missing.length > 0) {
    fail(`the turn-1 answer does not list every failing test (missing: ${missing.join(", ")})`);
  } else {
    const [first, second, third] = positions.sort((a, b) => a.at - b.at).map((entry) => entry.key);
    const label = (key) => `${FAILURES[key].file} (listed ${[first, second, third].indexOf(key) + 1}.)`;

    // Turn 2: only the second listed failure is fixed.
    expect(await isFixed(second, turnFile(2, FAILURES[second].file)), `turn 2 did not fix the second listed failure, ${label(second)}`);
    for (const key of [first, third]) {
      expect(turnFile(2, FAILURES[key].file) === headFile(FAILURES[key].file), `turn 2 changed ${label(key)}; only the second listed failure was to be fixed`);
    }
    // Turn 3 (final state): first and second fixed, third untouched.
    for (const key of [first, second]) {
      expect(await isFixed(key, read(FAILURES[key].file)), `${label(key)} is not fixed at the end`);
    }
    expect(read(FAILURES[third].file) === headFile(FAILURES[third].file), `${label(third)} was changed; it was to be left alone`);
  }
}

finish();
