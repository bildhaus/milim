// Wrong: fixes the borrow error and makes the visible sort test pass with a
// tuple sort key, but leaves Version's ordering (and so `latest`) broken.
import { fixBorrow, replace } from "./edits.mjs";

fixBorrow();
replace(
  "releases.sort_by_key(|release| release.version);",
  "releases.sort_by_key(|release| (release.version.major, release.version.minor, release.version.patch));",
);
