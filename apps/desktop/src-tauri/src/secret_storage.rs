use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use keyring::{Entry, Error as KeyringError};
use milim_core::{Error, Result};
use milim_storage::{create_private_file, EncryptedStore};
use milim_tools::atomic_write;
use serde::Serialize;
use sha2::{Digest, Sha256};

const SERVICE_NAME: &str = "com.milim.desktop";
const FALLBACK_FILE: &str = "desktop-storage.key";
const RETAIN_NATIVE_RECOVERY_KEY: bool = cfg!(target_os = "macos");

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretStorageMode {
    Native,
    RestrictedFile,
    Unavailable,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretStorageStatus {
    pub mode: SecretStorageMode,
    pub detail: String,
}

pub struct SecretStorage {
    pub encryption: Option<EncryptedStore>,
    pub status: SecretStorageStatus,
}

pub fn initialize(root: &Path) -> SecretStorage {
    match initialize_inner(root) {
        Ok((key, status)) => SecretStorage {
            encryption: Some(EncryptedStore::from_key(&key)),
            status,
        },
        Err(error) => {
            tracing::warn!("desktop secret storage unavailable: {error}");
            SecretStorage {
                encryption: None,
                status: SecretStorageStatus {
                    mode: SecretStorageMode::Unavailable,
                    detail: format!(
                        "Secret-backed integrations are disabled; existing files were preserved. {error}"
                    ),
                },
            }
        }
    }
}

fn initialize_inner(root: &Path) -> Result<([u8; 32], SecretStorageStatus)> {
    std::fs::create_dir_all(root)?;
    let canonical = std::fs::canonicalize(root)?;
    let account = format!(
        "storage-master-v1:{}",
        hex_digest(canonical.as_os_str().as_encoded_bytes())
    );
    let fallback_path = root.join(FALLBACK_FILE);
    let entry = Entry::new(SERVICE_NAME, &account);
    let (key, mut status) = match entry {
        Ok(entry) => resolve_with_native_store(
            &entry,
            &fallback_path,
            RETAIN_NATIVE_RECOVERY_KEY,
        )?,
        Err(error) => restricted_fallback(
            &fallback_path,
            format!("OS credential vault unavailable: {error}"),
        )?,
    };
    migrate_legacy_secrets(root, &key)?;
    if matches!(status.mode, SecretStorageMode::Native) && fallback_path.exists() {
        status = restricted_status(
            "The OS credential vault is active, but the restricted fallback file could not be removed.",
        );
    }
    Ok((key, status))
}

fn resolve_with_native_store(
    entry: &Entry,
    fallback_path: &Path,
    retain_recovery_key: bool,
) -> Result<([u8; 32], SecretStorageStatus)> {
    match entry.get_secret() {
        Ok(bytes) => {
            let native_key = key_from_bytes(&bytes, "native credential")?;
            if retain_recovery_key {
                let key = if fallback_path.exists() {
                    let fallback_key = read_restricted_key(fallback_path)?;
                    if fallback_key == native_key {
                        native_key
                    } else {
                        reconcile_native_and_recovery_keys(
                            entry,
                            fallback_path,
                            native_key,
                            fallback_key,
                        )?
                    }
                } else {
                    ensure_fallback(fallback_path, &native_key)?;
                    native_key
                };
                return Ok((key, native_recovery_status()));
            }
            if fallback_path.exists() {
                let fallback_key = read_restricted_key(fallback_path)?;
                if fallback_key != native_key {
                    return Err(Error::Other(
                        "native and restricted-file master keys differ; refusing to guess".into(),
                    ));
                }
                if let Err(error) = std::fs::remove_file(fallback_path) {
                    tracing::warn!("failed to remove desktop secret fallback: {error}");
                    return Ok((
                        native_key,
                        restricted_status(
                            "Secrets are encrypted, but a matching restricted fallback key remains on disk.",
                        ),
                    ));
                }
            }
            Ok((native_key, native_status()))
        }
        Err(KeyringError::NoEntry) => {
            let key = if fallback_path.exists() {
                read_restricted_key(fallback_path)?
            } else {
                EncryptedStore::random_key()
            };
            if retain_recovery_key {
                ensure_fallback(fallback_path, &key)?;
            }
            match store_and_verify(entry, &key) {
                Ok(()) => {
                    if retain_recovery_key {
                        return Ok((key, native_recovery_status()));
                    }
                    if fallback_path.exists() {
                        if let Err(error) = std::fs::remove_file(fallback_path) {
                            tracing::warn!("failed to remove desktop secret fallback: {error}");
                            return Ok((
                                key,
                                restricted_status(
                                    "The OS credential vault is active, but a matching restricted fallback key remains on disk.",
                                ),
                            ));
                        }
                    }
                    Ok((key, native_status()))
                }
                Err(error) => {
                    tracing::warn!("failed to initialize native credential vault: {error}");
                    ensure_fallback(fallback_path, &key)?;
                    Ok((
                        key,
                        restricted_status(
                            "The OS credential vault was unavailable, so the encryption key is stored in a restricted local file.",
                        ),
                    ))
                }
            }
        }
        Err(error) => restricted_fallback(
            fallback_path,
            format!("OS credential vault unavailable: {error}"),
        ),
    }
}

fn restricted_fallback(
    fallback_path: &Path,
    warning: String,
) -> Result<([u8; 32], SecretStorageStatus)> {
    tracing::warn!("{warning}");
    let key = if fallback_path.exists() {
        read_restricted_key(fallback_path)?
    } else {
        let key = EncryptedStore::random_key();
        create_private_file(fallback_path, &key)?;
        key
    };
    Ok((
        key,
        restricted_status(
            "The OS credential vault is unavailable, so the encryption key is stored in a restricted local file.",
        ),
    ))
}

fn ensure_fallback(path: &Path, key: &[u8; 32]) -> Result<()> {
    if path.exists() {
        let existing = read_restricted_key(path)?;
        if existing != *key {
            return Err(Error::Other(
                "existing restricted-file master key differs from the selected key".into(),
            ));
        }
        return Ok(());
    }
    create_private_file(path, key)
}

fn replace_fallback(path: &Path, key: &[u8; 32]) -> Result<()> {
    if !path.exists() {
        return create_private_file(path, key);
    }
    read_restricted_key(path)?;
    atomic_write(path, key)?;
    read_restricted_key(path).map(|_| ())
}

fn reconcile_native_and_recovery_keys(
    entry: &Entry,
    fallback_path: &Path,
    native_key: [u8; 32],
    fallback_key: [u8; 32],
) -> Result<[u8; 32]> {
    let root = fallback_path
        .parent()
        .ok_or_else(|| Error::InvalidRequest("desktop recovery key has no parent directory".into()))?;
    let native_matches = key_opens_existing_encrypted_state(root, &native_key)?;
    let fallback_matches = key_opens_existing_encrypted_state(root, &fallback_key)?;
    match (native_matches, fallback_matches) {
        (Some(true), Some(false)) => {
            replace_fallback(fallback_path, &native_key)?;
            tracing::warn!(
                "repaired a stale desktop recovery key from the validated native credential"
            );
            Ok(native_key)
        }
        (Some(false), Some(true)) => {
            store_and_verify(entry, &fallback_key)?;
            tracing::warn!(
                "repaired a stale native credential from the validated desktop recovery key"
            );
            Ok(fallback_key)
        }
        (None, None) => {
            replace_fallback(fallback_path, &native_key)?;
            Ok(native_key)
        }
        _ => Err(Error::Other(
            "native and recovery master keys differ, and encrypted state cannot identify one consistent key"
                .into(),
        )),
    }
}

fn key_opens_existing_encrypted_state(root: &Path, key: &[u8; 32]) -> Result<Option<bool>> {
    let encryption = EncryptedStore::from_key(key);
    let paths = [
        root.join("google-workspace.enc"),
        root.join("providers.enc"),
        root.join("mcp-secrets.enc"),
        root.join("config").join("mobile-companion.enc"),
    ];
    let mut found = false;
    for path in paths {
        let data = match std::fs::read(&path) {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        found = true;
        if encryption.decrypt(&data).is_err() {
            return Ok(Some(false));
        }
    }
    Ok(found.then_some(true))
}

fn store_and_verify(entry: &Entry, key: &[u8; 32]) -> Result<()> {
    entry
        .set_secret(key)
        .map_err(|error| Error::Other(format!("store native credential: {error}")))?;
    let stored = entry
        .get_secret()
        .map_err(|error| Error::Other(format!("verify native credential: {error}")))?;
    if stored != key {
        return Err(Error::Other(
            "native credential read-back did not match".into(),
        ));
    }
    Ok(())
}

fn native_status() -> SecretStorageStatus {
    #[cfg(target_os = "windows")]
    let detail = "The desktop encryption key is stored in Windows Credential Manager.";
    #[cfg(target_os = "macos")]
    let detail = "The desktop encryption key is stored in macOS Keychain.";
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let detail = "The desktop encryption key is stored in the OS credential vault.";
    SecretStorageStatus {
        mode: SecretStorageMode::Native,
        detail: detail.into(),
    }
}

fn restricted_status(detail: &str) -> SecretStorageStatus {
    SecretStorageStatus {
        mode: SecretStorageMode::RestrictedFile,
        detail: detail.into(),
    }
}

fn native_recovery_status() -> SecretStorageStatus {
    restricted_status(
        "The encryption key is stored in macOS Keychain with a matching owner-only local recovery key so encrypted state survives local app rebuilds.",
    )
}

fn hex_digest(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn read_key(path: &Path) -> Result<[u8; 32]> {
    key_from_bytes(&std::fs::read(path)?, &path.display().to_string())
}

fn read_restricted_key(path: &Path) -> Result<[u8; 32]> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)?.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(Error::Other(format!(
                "{} is readable by users other than its owner",
                path.display()
            )));
        }
    }
    read_key(path)
}

fn key_from_bytes(bytes: &[u8], label: &str) -> Result<[u8; 32]> {
    if bytes.len() != 32 {
        return Err(Error::Other(format!(
            "invalid {label} length: expected 32 bytes, got {}",
            bytes.len()
        )));
    }
    let mut key = [0_u8; 32];
    key.copy_from_slice(bytes);
    Ok(key)
}

fn migrate_legacy_secrets(root: &Path, master_key: &[u8; 32]) -> Result<()> {
    let encryption = EncryptedStore::from_key(master_key);
    for (key, data) in [
        ("google-workspace.key", "google-workspace.enc"),
        ("providers.key", "providers.enc"),
        ("mcp-secrets.key", "mcp-secrets.enc"),
    ] {
        migrate_encrypted_file(&root.join(key), &root.join(data), master_key, &encryption)?;
    }
    migrate_mobile_companion(root, &encryption)
}

fn migrate_encrypted_file(
    legacy_key_path: &Path,
    data_path: &Path,
    master_key: &[u8; 32],
    encryption: &EncryptedStore,
) -> Result<()> {
    let data = match std::fs::read(data_path) {
        Ok(data) => Some(data),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let Some(data) = data else {
        if legacy_key_path.exists() {
            std::fs::remove_file(legacy_key_path)?;
        }
        return Ok(());
    };

    if encryption.decrypt(&data).is_ok() {
        if legacy_key_path.exists() {
            std::fs::remove_file(legacy_key_path)?;
        }
        return Ok(());
    }

    let legacy_key = read_key(legacy_key_path)?;
    if legacy_key == *master_key {
        return Err(Error::Other(format!(
            "{} cannot be decrypted with its recorded master key",
            data_path.display()
        )));
    }
    let plaintext = EncryptedStore::from_key(&legacy_key).decrypt(&data)?;
    let migrated = encryption.encrypt(&plaintext)?;
    if encryption.decrypt(&migrated)? != plaintext {
        return Err(Error::Other(format!(
            "failed to verify migrated {}",
            data_path.display()
        )));
    }
    atomic_write(data_path, &migrated)?;
    let written = std::fs::read(data_path)?;
    if encryption.decrypt(&written)? != plaintext {
        return Err(Error::Other(format!(
            "failed to verify written {}",
            data_path.display()
        )));
    }
    std::fs::remove_file(legacy_key_path)?;
    Ok(())
}

fn migrate_mobile_companion(root: &Path, encryption: &EncryptedStore) -> Result<()> {
    let plain_path = root.join("config").join("mobile-companion.json");
    let encrypted_path = root.join("config").join("mobile-companion.enc");
    if encrypted_path.exists() {
        let data = std::fs::read(&encrypted_path)?;
        let plaintext = encryption.decrypt(&data)?;
        validate_mobile_companion(&plaintext, &encrypted_path)?;
        if plain_path.exists() {
            std::fs::remove_file(plain_path)?;
        }
        return Ok(());
    }
    let plaintext = match std::fs::read(&plain_path) {
        Ok(data) => data,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    validate_mobile_companion(&plaintext, &plain_path)?;
    let migrated = encryption.encrypt(&plaintext)?;
    if encryption.decrypt(&migrated)? != plaintext {
        return Err(Error::Other(
            "failed to verify migrated mobile companion state".into(),
        ));
    }
    atomic_write(&encrypted_path, &migrated)?;
    let written = std::fs::read(&encrypted_path)?;
    let decoded = encryption.decrypt(&written)?;
    validate_mobile_companion(&decoded, &encrypted_path)?;
    if decoded != plaintext {
        return Err(Error::Other(
            "written mobile companion state did not match".into(),
        ));
    }
    std::fs::remove_file(plain_path)?;
    Ok(())
}

fn validate_mobile_companion(data: &[u8], path: &Path) -> Result<()> {
    milim_server::companion::validate_mobile_companion_persistence(data)
        .map_err(|error| Error::Other(format!("invalid {}: {error}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use keyring::mock::MockCredential;

    fn root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "milim-secret-storage-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn migrates_legacy_encrypted_file_and_is_idempotent() {
        let root = root("encrypted");
        let legacy_key = [7_u8; 32];
        let master_key = [9_u8; 32];
        let key_path = root.join("providers.key");
        let data_path = root.join("providers.enc");
        std::fs::write(&key_path, legacy_key).unwrap();
        let plaintext = br#"[{"id":"provider"}]"#;
        std::fs::write(
            &data_path,
            EncryptedStore::from_key(&legacy_key)
                .encrypt(plaintext)
                .unwrap(),
        )
        .unwrap();
        let master = EncryptedStore::from_key(&master_key);

        migrate_encrypted_file(&key_path, &data_path, &master_key, &master).unwrap();
        assert!(!key_path.exists());
        assert_eq!(
            master.decrypt(&std::fs::read(&data_path).unwrap()).unwrap(),
            plaintext
        );
        migrate_encrypted_file(&key_path, &data_path, &master_key, &master).unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn promotes_fallback_to_native_without_changing_the_key() {
        let root = root("promote");
        let fallback = root.join(FALLBACK_FILE);
        let key = [11_u8; 32];
        create_private_file(&fallback, &key).unwrap();
        let entry = Entry::new_with_credential(Box::<MockCredential>::default());

        let (resolved, status) = resolve_with_native_store(&entry, &fallback, false).unwrap();

        assert_eq!(resolved, key);
        assert!(matches!(status.mode, SecretStorageMode::Native));
        assert_eq!(entry.get_secret().unwrap(), key);
        assert!(!fallback.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn retained_recovery_key_survives_native_store_unavailability() {
        let root = root("native-recovery");
        let fallback = root.join(FALLBACK_FILE);
        let entry = Entry::new_with_credential(Box::<MockCredential>::default());

        let (first_key, first_status) =
            resolve_with_native_store(&entry, &fallback, true).unwrap();
        assert!(matches!(first_status.mode, SecretStorageMode::RestrictedFile));
        assert_eq!(read_restricted_key(&fallback).unwrap(), first_key);

        let companion_path = root.join("config").join("mobile-companion.enc");
        let bridge = milim_server::companion::MobileCompanionBridge::persistent_encrypted(
            companion_path.clone(),
            EncryptedStore::from_key(&first_key),
        )
        .unwrap();
        bridge.set_enabled(true, 1);
        let pairing = bridge.start_pairing(2).unwrap();
        let secret = pairing.path.split("secret=").nth(1).unwrap().to_string();
        let paired = bridge
            .pair_device(
                milim_server::companion::MobilePairRequest {
                    pair_id: pairing.id,
                    secret,
                    device_name: Some("Mac test phone".into()),
                },
                3,
                None,
            )
            .unwrap();

        let credential = entry
            .get_credential()
            .downcast_ref::<MockCredential>()
            .unwrap();
        credential.set_error(KeyringError::NoStorageAccess(Box::new(
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "vault unavailable"),
        )));

        let (reloaded_key, reloaded_status) =
            resolve_with_native_store(&entry, &fallback, true).unwrap();
        assert_eq!(reloaded_key, first_key);
        assert!(matches!(
            reloaded_status.mode,
            SecretStorageMode::RestrictedFile
        ));
        let reloaded = milim_server::companion::MobileCompanionBridge::persistent_encrypted(
            companion_path,
            EncryptedStore::from_key(&reloaded_key),
        )
        .unwrap();
        let status = reloaded.status(4);
        assert!(status.enabled);
        assert_eq!(status.devices.len(), 1);
        assert_eq!(status.devices[0].id, paired.device_id);
        assert!(reloaded
            .authenticate_device(&paired.device_key, 5)
            .is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reconciles_native_and_recovery_keys_against_encrypted_state() {
        let native_root = root("native-reconcile");
        let native_fallback = native_root.join(FALLBACK_FILE);
        let native_key = [21_u8; 32];
        let stale_key = [22_u8; 32];
        create_private_file(&native_fallback, &stale_key).unwrap();
        let native_config = native_root.join("config");
        std::fs::create_dir_all(&native_config).unwrap();
        std::fs::write(
            native_config.join("mobile-companion.enc"),
            EncryptedStore::from_key(&native_key)
                .encrypt(br#"{"enabled":true,"devices":[]}"#)
                .unwrap(),
        )
        .unwrap();
        let native_entry = Entry::new_with_credential(Box::<MockCredential>::default());
        native_entry.set_secret(&native_key).unwrap();

        let (recovered_native, _) =
            resolve_with_native_store(&native_entry, &native_fallback, true).unwrap();
        assert_eq!(recovered_native, native_key);
        assert_eq!(read_restricted_key(&native_fallback).unwrap(), native_key);

        let recovery_root = root("recovery-reconcile");
        let recovery_fallback = recovery_root.join(FALLBACK_FILE);
        let recovery_key = [31_u8; 32];
        let stale_native_key = [32_u8; 32];
        create_private_file(&recovery_fallback, &recovery_key).unwrap();
        let recovery_config = recovery_root.join("config");
        std::fs::create_dir_all(&recovery_config).unwrap();
        std::fs::write(
            recovery_config.join("mobile-companion.enc"),
            EncryptedStore::from_key(&recovery_key)
                .encrypt(br#"{"enabled":true,"devices":[]}"#)
                .unwrap(),
        )
        .unwrap();
        let recovery_entry = Entry::new_with_credential(Box::<MockCredential>::default());
        recovery_entry.set_secret(&stale_native_key).unwrap();

        let (recovered_fallback, _) =
            resolve_with_native_store(&recovery_entry, &recovery_fallback, true).unwrap();
        assert_eq!(recovered_fallback, recovery_key);
        assert_eq!(recovery_entry.get_secret().unwrap(), recovery_key);

        let _ = std::fs::remove_dir_all(native_root);
        let _ = std::fs::remove_dir_all(recovery_root);
    }

    #[test]
    fn corrupt_legacy_data_is_preserved() {
        let root = root("corrupt");
        let key_path = root.join("providers.key");
        let data_path = root.join("providers.enc");
        std::fs::write(&key_path, [7_u8; 32]).unwrap();
        std::fs::write(&data_path, b"corrupt").unwrap();
        let before_key = std::fs::read(&key_path).unwrap();
        let before_data = std::fs::read(&data_path).unwrap();

        assert!(migrate_encrypted_file(
            &key_path,
            &data_path,
            &[9_u8; 32],
            &EncryptedStore::from_key(&[9_u8; 32])
        )
        .is_err());
        assert_eq!(std::fs::read(&key_path).unwrap(), before_key);
        assert_eq!(std::fs::read(&data_path).unwrap(), before_data);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn migrates_mobile_plaintext() {
        let root = root("mobile");
        let config = root.join("config");
        std::fs::create_dir_all(&config).unwrap();
        let plain_path = config.join("mobile-companion.json");
        let encrypted_path = config.join("mobile-companion.enc");
        let plaintext = br#"{"enabled":true,"devices":[]}"#;
        std::fs::write(&plain_path, plaintext).unwrap();
        let encryption = EncryptedStore::from_key(&[4_u8; 32]);

        migrate_mobile_companion(&root, &encryption).unwrap();
        assert!(!plain_path.exists());
        assert_eq!(
            encryption
                .decrypt(&std::fs::read(encrypted_path).unwrap())
                .unwrap(),
            plaintext
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_mobile_plaintext_is_preserved() {
        let root = root("invalid-mobile");
        let config = root.join("config");
        std::fs::create_dir_all(&config).unwrap();
        let plain_path = config.join("mobile-companion.json");
        let encrypted_path = config.join("mobile-companion.enc");
        std::fs::write(&plain_path, b"[]").unwrap();

        assert!(migrate_mobile_companion(&root, &EncryptedStore::from_key(&[4_u8; 32])).is_err());
        assert_eq!(std::fs::read(&plain_path).unwrap(), b"[]");
        assert!(!encrypted_path.exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
