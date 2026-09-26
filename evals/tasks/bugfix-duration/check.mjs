import { attempt, expect, expectEqual, expectOnlyChanged, finish, headFile, load, read, runNodeTests } from "../../lib/check.mjs";

expect(read("test/duration.test.js") === headFile("test/duration.test.js"), "tests were modified");
expectOnlyChanged((path) => path.startsWith("src/"));
runNodeTests("test");

await attempt("hidden cases", async () => {
  const { parseDuration, formatDuration } = await load("src/duration.js");
  expectEqual(parseDuration("1d2h3m4s5ms"), 93_784_005, "all units");
  expectEqual(parseDuration(" 1m 1s "), 61_000, "surrounding whitespace");
  expectEqual(parseDuration("10m5ms"), 600_005, "m followed by ms");
  expectEqual(formatDuration(600_005), "10m5ms", "format unchanged");
  for (const bad of ["1x", "h1", "1h30", "1.5h", "--1h", "1h 30 m"]) {
    let threw = false;
    try {
      parseDuration(bad);
    } catch (error) {
      threw = error instanceof TypeError;
    }
    expect(threw, `expected TypeError for ${JSON.stringify(bad)}`);
  }
});

finish();
