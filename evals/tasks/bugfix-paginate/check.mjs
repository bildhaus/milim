import { attempt, expect, expectEqual, expectOnlyChanged, finish, headFile, load, read, runNodeTests } from "../../lib/check.mjs";

expect(read("test/paginate.test.js") === headFile("test/paginate.test.js"), "tests were modified");
expectOnlyChanged(["src/paginate.js"]);
runNodeTests("test");

await attempt("hidden cases", async () => {
  const { paginate } = await load("src/paginate.js");
  const items = Array.from({ length: 7 }, (_, index) => index);
  expectEqual(paginate(items, 2, 3).items, [3, 4, 5], "page 2 of 7 by 3");
  expectEqual(paginate(items, 3, 3), { items: [6], page: 3, pageSize: 3, totalItems: 7, totalPages: 3, hasNext: false }, "last page");
  expectEqual(paginate(items, 5, 3).items, [], "past the end");
  expectEqual(paginate([], 1, 5).totalPages, 0, "empty list pages");
  expectEqual(paginate(items, 1, 7).hasNext, false, "single exact page");
  expectEqual(paginate(items, 1, 2).hasNext, true, "has next");
});

finish();
