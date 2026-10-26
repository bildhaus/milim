// Wrong: "fixes" the borrow error by no longer logging bulk yanks.
import { reorderFields, replace } from "./edits.mjs";

replace(`                self.record(format!("yanked {}", release.version));\n`, "");
reorderFields();
