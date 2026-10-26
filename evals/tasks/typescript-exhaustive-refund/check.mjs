import { spawnSync } from "node:child_process";
import {
  attempt,
  expect,
  expectEqual,
  expectOnlyChanged,
  finish,
  findTsc,
  headFile,
  load,
  read,
  repo,
  runNodeTests,
  skip,
  stripComments,
} from "../../lib/check.mjs";

if (!process.features?.typescript) skip(`Node ${process.version} does not strip TypeScript types by default`);
const tsc = findTsc();
if (!tsc) skip("the TypeScript compiler (tsc) is not installed");

const SOURCES = ["src/events.ts", "src/format.ts", "src/summary.ts"];
for (const path of ["test/ledger.test.ts", "tsconfig.json", "package.json"]) {
  expect(read(path) === headFile(path), `${path} was modified`);
}
expectOnlyChanged(SOURCES);

// No escape hatches: the point is to make the union exhaustive for real.
for (const path of SOURCES) {
  const code = stripComments(read(path));
  expect(!/\bany\b/.test(code), `${path} uses any`);
  expect(!/\bas\s+(?!const\b)[\w{[(]/.test(code), `${path} uses a type assertion (as)`);
  expect(!/[\w)\]]!(?=[.[\s;,)])/.test(code), `${path} uses a non-null assertion (!)`);
  expect(!/@ts-(ignore|expect-error|nocheck)/.test(read(path)), `${path} uses a @ts- comment`);
}
expect(/default:\s*return\s+assertNever\(event\)/.test(stripComments(read("src/events.ts"))), "applyEvent no longer ends with the assertNever exhaustiveness check");

const [command, ...prefix] = tsc;
const typecheck = spawnSync(command, [...prefix, "--noEmit", "-p", "."], { cwd: repo, encoding: "utf8", timeout: 90_000 });
expect(typecheck.status === 0, `tsc --noEmit failed: ${`${typecheck.stdout}${typecheck.stderr}`.trim().split("\n").slice(0, 4).join(" | ")}`);

runNodeTests("test", /\.test\.ts$/);

await attempt("hidden refund cases", async () => {
  const { balanceAfter } = await load("src/events.ts");
  const { describe } = await load("src/format.ts");
  const { totalsByKind } = await load("src/summary.ts");
  expectEqual(balanceAfter([{ kind: "refund", amount: 1_500 }], 100), 1_600, "refund without a fee");
  expectEqual(balanceAfter([{ kind: "refund", amount: 1_500, fee: 0 }]), 1_500, "refund with a zero fee");
  expectEqual(describe({ kind: "refund", amount: 1_500 }), "refund 15.00", "statement line without a fee");
  expectEqual(describe({ kind: "refund", amount: 99, fee: 5 }), "refund 0.99 (fee 0.05)", "statement line with a fee");
  expectEqual(
    totalsByKind([{ kind: "refund", amount: 700, fee: 100 }, { kind: "refund", amount: 300 }]),
    { deposit: 0, withdrawal: 0, transfer: 0, refund: 1_000 },
    "refund totals are gross",
  );
});

finish();
