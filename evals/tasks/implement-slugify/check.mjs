import { attempt, expect, expectEqual, finish, load } from "../../lib/check.mjs";

function throwsType(fn, Type) {
  try {
    fn();
  } catch (error) {
    return error instanceof Type;
  }
  return false;
}

await attempt("slugify", async () => {
  const { slugify } = await load("src/slugify.js");
  const cases = [
    ["Hello, World!", undefined, "hello-world"],
    ["  Rock & Roll  ", undefined, "rock-and-roll"],
    ["Crème Brûlée", { separator: "_" }, "creme_brulee"],
    ["The quick brown fox", { maxLength: 13 }, "the-quick"],
    ["The quick brown fox", { maxLength: 9 }, "the-quick"],
    ["The quick brown fox", { maxLength: 15 }, "the-quick-brown"],
    ["Supercalifragilistic", { maxLength: 5 }, "super"],
    ["!!!", undefined, "n-a"],
    ["", { separator: "." }, "n.a"],
    ["R&D", undefined, "r-and-d"],
    ["Ünïcödé -- 2024 édition", undefined, "unicode-2024-edition"],
    ["a_b.c-d", { separator: "." }, "a.b.c.d"],
    ["x".repeat(100), undefined, "x".repeat(80)],
    ["Tabs\tand\nnewlines", undefined, "tabs-and-newlines"],
  ];
  for (const [input, options, expected] of cases) {
    expectEqual(slugify(input, options), expected, `slugify(${JSON.stringify(input)}, ${JSON.stringify(options)})`);
  }
  expect(throwsType(() => slugify(42), TypeError), "non-string input must throw TypeError");
  expect(throwsType(() => slugify("a", { separator: "--" }), RangeError), "bad separator must throw RangeError");
  expect(throwsType(() => slugify("a", { separator: "+" }), RangeError), "unsupported separator must throw RangeError");
  expect(throwsType(() => slugify("a", { maxLength: 0 }), RangeError), "maxLength 0 must throw RangeError");
  expect(throwsType(() => slugify("a", { maxLength: 2.5 }), RangeError), "fractional maxLength must throw RangeError");
});

finish();
