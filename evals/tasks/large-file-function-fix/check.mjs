import { attempt, expect, expectEqual, expectOnlyChanged, finish, headFile, lines, load, read } from "../../lib/check.mjs";

const PATH = "src/formatters.js";
expectOnlyChanged([PATH]);

/** [start, end) of formatBytes including its JSDoc block. */
function span(all) {
  const start = all.findIndex((line) => line.startsWith("export function formatBytes("));
  let doc = start;
  while (doc > 0 && !all[doc - 1].startsWith("/**")) doc -= 1;
  const end = all.findIndex((line, index) => index > start && line === "}");
  return start < 0 || end < 0 ? null : [doc - 1, end + 1];
}

const before = lines(headFile(PATH));
const after = lines(read(PATH));
const old = span(before);
const now = span(after);
if (expect(now !== null, "formatBytes not found")) {
  expectEqual(after.slice(0, now[0]), before.slice(0, old[0]), "lines before formatBytes");
  expect(
    JSON.stringify(after.slice(now[1])) === JSON.stringify(before.slice(old[1])),
    "lines after formatBytes changed",
  );
}

await attempt("formatBytes", async () => {
  const module = await load(PATH);
  const { formatBytes } = module;
  const cases = [
    [0, "0 B"],
    [512, "512 B"],
    [1023, "1023 B"],
    [1024, "1.0 KiB"],
    [1536, "1.5 KiB"],
    [1_047_552, "1023.0 KiB"],
    [1_048_576, "1.0 MiB"],
    [1_500_000, "1.4 MiB"],
    [5 * 1024 ** 3, "5.0 GiB"],
    [2.5 * 1024 ** 4, "2.5 TiB"],
    [3 * 1024 ** 5, "3072.0 TiB"],
  ];
  for (const [input, expected] of cases) expectEqual(formatBytes(input), expected, `formatBytes(${input})`);
  let threw = false;
  try {
    formatBytes(-1);
  } catch (error) {
    threw = error instanceof RangeError;
  }
  expect(threw, "negative input throws RangeError");
  expectEqual(module.formatCelsiusAsFahrenheitColumn8(100), "212.0 °F", "neighbor function intact");
});

finish();
