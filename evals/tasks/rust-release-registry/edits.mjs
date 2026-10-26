// Shared by the solution scripts: text edits to src/lib.rs.
import { readFileSync, writeFileSync } from "node:fs";

export function replace(from, to) {
  const source = readFileSync("src/lib.rs", "utf8");
  if (!source.includes(from)) throw new Error(`src/lib.rs: expected text not found: ${from.slice(0, 60)}`);
  writeFileSync("src/lib.rs", source.replace(from, () => to));
}

/** The borrow fix: push to the disjoint `log` field instead of calling `&mut self`. */
export const fixBorrow = () =>
  replace(`                self.record(format!("yanked {}", release.version));`, `                self.log.push(format!("yanked {}", release.version));`);

/** The ordering fix: declare the fields in comparison order. */
export const reorderFields = () =>
  replace(
    `pub struct Version {
    pub patch: u32,
    pub minor: u32,
    pub major: u32,
}`,
    `pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}`,
  );
