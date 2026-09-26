import { changedFiles, exists, expect, expectEqual, finish, read } from "../../lib/check.mjs";

const changes = changedFiles();
expectEqual(changes, ["ANSWER.json"], "only ANSWER.json may be added");

if (expect(exists("ANSWER.json"), "ANSWER.json missing")) {
  let answer = null;
  try {
    answer = JSON.parse(read("ANSWER.json"));
  } catch (error) {
    expect(false, `ANSWER.json is not valid JSON: ${error.message}`);
  }
  if (answer) {
    expectEqual(answer.function, "goldRate", "function");
    expectEqual(String(answer.file).replace(/^\.\//, ""), "src/pricing/gold.js", "file");
    expectEqual(Number(answer.max_percent), 22, "max_percent");
  }
}

finish();
