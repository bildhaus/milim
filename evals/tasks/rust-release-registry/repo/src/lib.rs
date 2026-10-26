//! A small registry of published releases of one package.

use std::fmt;

/// A `major.minor.patch` version, ordered the way semantic versioning
/// orders releases: by major, then minor, then patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub patch: u32,
    pub minor: u32,
    pub major: u32,
}

impl Version {
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Version { patch, minor, major }
    }

    /// Parse `"1.2.3"`. Exactly three dot-separated decimal numbers.
    pub fn parse(text: &str) -> Result<Version, RegistryError> {
        let parts: Vec<&str> = text.trim().split('.').collect();
        if parts.len() != 3 {
            return Err(RegistryError::InvalidVersion(text.to_string()));
        }
        let mut numbers = [0u32; 3];
        for (slot, part) in numbers.iter_mut().zip(&parts) {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(RegistryError::InvalidVersion(text.to_string()));
            }
            *slot = part
                .parse()
                .map_err(|_| RegistryError::InvalidVersion(text.to_string()))?;
        }
        Ok(Version::new(numbers[0], numbers[1], numbers[2]))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    InvalidVersion(String),
    AlreadyPublished(Version),
    NotFound(Version),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub notes: String,
    pub yanked: bool,
}

#[derive(Debug, Default)]
pub struct Registry {
    releases: Vec<Release>,
    log: Vec<String>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn publish(&mut self, version: &str, notes: &str) -> Result<(), RegistryError> {
        let version = Version::parse(version)?;
        if self.releases.iter().any(|release| release.version == version) {
            return Err(RegistryError::AlreadyPublished(version));
        }
        self.releases.push(Release {
            version,
            notes: notes.to_string(),
            yanked: false,
        });
        self.record(format!("published {version}"));
        Ok(())
    }

    pub fn yank(&mut self, version: &str) -> Result<(), RegistryError> {
        let version = Version::parse(version)?;
        let release = self
            .releases
            .iter_mut()
            .find(|release| release.version == version)
            .ok_or(RegistryError::NotFound(version))?;
        release.yanked = true;
        self.record(format!("yanked {version}"));
        Ok(())
    }

    /// Yank every release older than `floor` that is not yanked yet, log
    /// each one, and return how many were yanked.
    pub fn yank_below(&mut self, floor: &str) -> Result<usize, RegistryError> {
        let floor = Version::parse(floor)?;
        let mut count = 0;
        for release in self.releases.iter_mut() {
            if release.version < floor && !release.yanked {
                release.yanked = true;
                self.record(format!("yanked {}", release.version));
                count += 1;
            }
        }
        Ok(count)
    }

    /// The newest release that is not yanked.
    pub fn latest(&self) -> Option<&Release> {
        self.releases
            .iter()
            .filter(|release| !release.yanked)
            .max_by_key(|release| release.version)
    }

    /// Every release, oldest first.
    pub fn sorted(&self) -> Vec<&Release> {
        let mut releases: Vec<&Release> = self.releases.iter().collect();
        releases.sort_by_key(|release| release.version);
        releases
    }

    pub fn log(&self) -> &[String] {
        &self.log
    }

    fn record(&mut self, message: String) {
        self.log.push(message);
    }
}
