//! Local machine identity: a persisted master secret used to mint msk-v1 keys.

use std::path::Path;

use rand::RngCore;

use milim_core::Result;
use milim_storage::create_private_file;

use crate::access_key::{mint_access_key, AccessKeyPayload};
use crate::crypto::{derive_address, generate_secret};

/// Parameters for minting an access key.
#[derive(Debug, Clone)]
pub struct MintParams {
    /// Audience address the key authorizes against.
    pub audience: String,
    /// Optional human label.
    pub label: Option<String>,
    /// Issued-at (unix seconds).
    pub iat: i64,
    /// Optional expiry (unix seconds).
    pub exp: Option<i64>,
    /// Monotonic counter (for revocation).
    pub cnt: u64,
    /// Random nonce.
    pub nonce: String,
}

/// A secp256k1 identity backed by a 32-byte secret.
pub struct LocalIdentity {
    secret: [u8; 32],
}

impl LocalIdentity {
    /// Build from a raw secret.
    pub fn from_secret(secret: [u8; 32]) -> Self {
        Self { secret }
    }

    /// Load the secret from `path`, generating and persisting one if absent.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut secret = [0u8; 32];
                secret.copy_from_slice(&bytes);
                return Ok(Self { secret });
            }
            Ok(bytes) => {
                return Err(milim_core::Error::Other(format!(
                    "invalid identity key length: expected 32 bytes, got {}",
                    bytes.len()
                )))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let secret = generate_secret();
        create_private_file(path, &secret)?;
        Ok(Self { secret })
    }

    /// This identity's checksummed address.
    pub fn address(&self) -> Result<String> {
        derive_address(&self.secret)
    }

    /// Mint an msk-v1 token with the given parameters.
    pub fn mint_token(&self, params: MintParams) -> Result<String> {
        let iss = self.address()?;
        let payload = AccessKeyPayload {
            aud: params.audience,
            cnt: params.cnt,
            exp: params.exp,
            iat: params.iat,
            iss,
            lbl: params.label,
            nonce: params.nonce,
        };
        mint_access_key(&self.secret, &payload)
    }
}

/// A random 32-hex-character nonce.
pub fn random_nonce() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access_key::AccessKeyValidator;

    #[test]
    fn mint_round_trips_through_validator() {
        let id = LocalIdentity::from_secret([3u8; 32]);
        let addr = id.address().unwrap();
        let token = id
            .mint_token(MintParams {
                audience: addr.clone(),
                label: Some("cli".into()),
                iat: 1_700_000_000,
                exp: None,
                cnt: 1,
                nonce: random_nonce(),
            })
            .unwrap();
        let validator = AccessKeyValidator::new(&addr, &addr).with_issuer(&addr);
        assert!(validator.validate(&token).is_valid());
    }

    #[test]
    fn load_or_create_is_stable() {
        let dir = std::env::temp_dir().join(format!("milim-id-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("master.key");
        let a = LocalIdentity::load_or_create(&path)
            .unwrap()
            .address()
            .unwrap();
        let b = LocalIdentity::load_or_create(&path)
            .unwrap()
            .address()
            .unwrap();
        assert_eq!(a, b);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
