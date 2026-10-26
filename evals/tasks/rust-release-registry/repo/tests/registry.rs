use release_registry::{Registry, RegistryError, Version};

fn registry(versions: &[&str]) -> Registry {
    let mut registry = Registry::new();
    for version in versions {
        registry.publish(version, "notes").unwrap();
    }
    registry
}

#[test]
fn parses_and_displays_versions() {
    assert_eq!(Version::parse("1.10.3").unwrap(), Version::new(1, 10, 3));
    assert_eq!(Version::new(2, 0, 1).to_string(), "2.0.1");
    assert!(matches!(Version::parse("1.2"), Err(RegistryError::InvalidVersion(_))));
}

#[test]
fn rejects_duplicate_publishes() {
    let mut registry = registry(&["1.0.0"]);
    assert_eq!(
        registry.publish("1.0.0", "again"),
        Err(RegistryError::AlreadyPublished(Version::new(1, 0, 0)))
    );
}

#[test]
fn sorts_releases_oldest_first() {
    let registry = registry(&["1.2.3", "2.0.0", "1.10.0", "0.9.12"]);
    let order: Vec<String> = registry.sorted().iter().map(|release| release.version.to_string()).collect();
    assert_eq!(order, ["0.9.12", "1.2.3", "1.10.0", "2.0.0"]);
}

#[test]
fn yanks_everything_below_a_floor() {
    let mut registry = registry(&["0.9.0", "1.0.0", "1.1.0"]);
    assert_eq!(registry.yank_below("1.1.0"), Ok(2));
    assert_eq!(registry.yank_below("1.1.0"), Ok(0));
}
