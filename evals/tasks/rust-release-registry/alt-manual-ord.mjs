// Another correct fix: collect the yanked versions before logging, and
// implement Ord by hand instead of reordering the fields.
import { replace } from "./edits.mjs";

replace(
  `        let mut count = 0;
        for release in self.releases.iter_mut() {
            if release.version < floor && !release.yanked {
                release.yanked = true;
                self.record(format!("yanked {}", release.version));
                count += 1;
            }
        }
        Ok(count)`,
  `        let mut yanked = Vec::new();
        for release in self.releases.iter_mut() {
            if release.version < floor && !release.yanked {
                release.yanked = true;
                yanked.push(release.version);
            }
        }
        for version in &yanked {
            self.record(format!("yanked {version}"));
        }
        Ok(yanked.len())`,
);
replace("#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]\npub struct Version", "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\npub struct Version");
replace(
  "impl fmt::Display for Version {",
  `impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {`,
);
