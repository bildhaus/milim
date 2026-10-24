//! Low-level crypto for milim-interoperable identity: keccak-256,
//! domain-separated recoverable secp256k1 signing, and EIP-55 addresses.
//!
//! Mirrors milim's `CryptoHelpers.swift` byte-for-byte:
//!   - Keccak-256 (legacy, 0x01 padding — NOT NIST SHA3).
//!   - Signed digest = `keccak256("\x19" + prefix + ":\n" + len + payload)`.
//!   - Signature = `r(32) || s(32) || v(1)`, where `v = recovery_id + 27`.
//!   - Address = EIP-55 checksum of `keccak256(uncompressed_pubkey[1..])[12..]`.

use k256::ecdsa::{RecoveryId, Signature, SigningKey, VerifyingKey};
use k256::elliptic_curve::Generate;
use milim_core::{Error, Result};
use sha3::{Digest, Keccak256};

/// Domain prefix used for msk-v1 access keys.
pub const ACCESS_PREFIX: &str = "Milim Signed Access";

/// Keccak-256 (legacy) of `data`.
pub fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// The domain-separated digest that is actually signed/recovered.
pub fn domain_hash(prefix: &str, payload: &[u8]) -> [u8; 32] {
    let mut buf = format!("\u{19}{prefix}:\n{}", payload.len()).into_bytes();
    buf.extend_from_slice(payload);
    keccak256(&buf)
}

/// Sign a payload with a domain prefix, returning a 65-byte recoverable sig.
pub fn sign_with_prefix(payload: &[u8], secret: &[u8], prefix: &str) -> Result<[u8; 65]> {
    let key = SigningKey::from_slice(secret).map_err(|e| Error::Other(format!("bad key: {e}")))?;
    let digest = domain_hash(prefix, payload);
    let (sig, recid) = key.sign_prehash_recoverable(&digest);
    let mut out = [0u8; 65];
    out[..64].copy_from_slice(&sig.to_bytes());
    out[64] = recid.to_byte() + 27;
    Ok(out)
}

/// Recover the signer's checksummed address from a payload + 65-byte signature.
pub fn recover_address(payload: &[u8], signature: &[u8], prefix: &str) -> Result<String> {
    if signature.len() != 65 {
        return Err(Error::Other("signature must be 65 bytes".to_string()));
    }
    let digest = domain_hash(prefix, payload);
    let recid = RecoveryId::from_byte(signature[64].wrapping_sub(27))
        .ok_or_else(|| Error::Other("invalid recovery id".to_string()))?;
    let sig = Signature::from_slice(&signature[..64])
        .map_err(|e| Error::Other(format!("bad signature: {e}")))?;
    let vk = VerifyingKey::recover_from_prehash(&digest, &sig, recid)
        .map_err(|e| Error::Other(format!("recovery failed: {e}")))?;
    Ok(address_from_verifying_key(&vk))
}

/// Derive the checksummed address for a secret key.
pub fn derive_address(secret: &[u8]) -> Result<String> {
    let key = SigningKey::from_slice(secret).map_err(|e| Error::Other(format!("bad key: {e}")))?;
    Ok(address_from_verifying_key(key.verifying_key()))
}

fn address_from_verifying_key(vk: &VerifyingKey) -> String {
    let point = vk.to_sec1_point(false); // 0x04 || X(32) || Y(32)
    let body = &point.as_bytes()[1..]; // drop 0x04
    let hash = keccak256(body);
    let raw: String = hash[12..].iter().map(|b| format!("{b:02x}")).collect();
    eip55_checksum(&raw)
}

/// EIP-55 mixed-case checksum encoding of a 40-char lowercase hex address.
pub fn eip55_checksum(raw_lower: &str) -> String {
    let hash_hex = hex::encode(keccak256(raw_lower.as_bytes()));
    let hash_bytes = hash_hex.as_bytes();
    let mut out = String::with_capacity(2 + raw_lower.len());
    out.push_str("0x");
    for (i, c) in raw_lower.chars().enumerate() {
        let nibble = (hash_bytes[i] as char).to_digit(16).unwrap_or(0);
        if nibble >= 8 {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Generate a random secp256k1 secret key (32 bytes).
pub fn generate_secret() -> [u8; 32] {
    SigningKey::generate().to_bytes().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_key_preserves_address_and_recoverable_signature() {
        // Public test key and signature recorded with k256 0.13.4 before migration.
        let mut secret = [0u8; 32];
        secret[31] = 1;
        let payload = b"milim identity compatibility";
        let address = "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf";
        let expected = hex::decode("77fd3f542e8d0e9dcdc64b3d5d6cd1cab6261da3beaf0802330c1095b1167c484f57c1bbc2b035d978ce29a9d6f450789d6aa84d43fad93a20eda489bb87f5141b").unwrap();
        assert_eq!(derive_address(&secret).unwrap(), address);
        assert_eq!(
            sign_with_prefix(payload, &secret, ACCESS_PREFIX).unwrap(),
            expected.as_slice()
        );
        assert_eq!(
            recover_address(payload, &expected, ACCESS_PREFIX).unwrap(),
            address
        );
    }

    #[test]
    fn sign_then_recover_matches_address() {
        let secret = generate_secret();
        let address = derive_address(&secret).unwrap();
        let payload = br#"{"hello":"world"}"#;
        let sig = sign_with_prefix(payload, &secret, ACCESS_PREFIX).unwrap();
        let recovered = recover_address(payload, &sig, ACCESS_PREFIX).unwrap();
        assert_eq!(recovered, address);
    }

    #[test]
    fn wrong_prefix_recovers_different_address() {
        let secret = generate_secret();
        let address = derive_address(&secret).unwrap();
        let payload = b"abc";
        let sig = sign_with_prefix(payload, &secret, ACCESS_PREFIX).unwrap();
        // Recovering under a different domain prefix must NOT yield the signer.
        let other = recover_address(payload, &sig, "Milim Signed Pairing").unwrap();
        assert_ne!(other, address);
    }

    #[test]
    fn address_is_checksummed_and_well_formed() {
        let addr = derive_address(&[0x11; 32]).unwrap();
        assert!(addr.starts_with("0x"));
        assert_eq!(addr.len(), 42);
        assert!(addr[2..].chars().all(|c| c.is_ascii_hexdigit()));
        // Deterministic for a fixed key.
        assert_eq!(derive_address(&[0x11; 32]).unwrap(), addr);
    }
}
