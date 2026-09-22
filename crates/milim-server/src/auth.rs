//! Bearer-token auth with loopback trust (Phase 1).
//!
//! Phase 2 replaces the static key-set with milim's `msk-v1` access-key
//! validation (secp256k1 ecrecover + whitelist/revocation).

use std::net::SocketAddr;

use axum::http::header::AUTHORIZATION;
use axum::http::HeaderMap;

use milim_core::Error;

use crate::error::ApiError;
use crate::state::AppState;

/// Authorize a request, or return a 401 `ApiError`.
///
/// Rules: if neither static keys nor an msk-v1 validator are configured, allow
/// everything (dev). Otherwise accept loopback peers when `trust_loopback` is
/// set, else require either a matching static `Authorization: Bearer <key>` or
/// a valid `msk-v1` access token.
pub fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
) -> Result<(), ApiError> {
    if state.api_keys.is_empty() && state.access_validator.is_none() {
        return Ok(());
    }
    if state.trust_loopback {
        if let Some(addr) = peer {
            if addr.ip().is_loopback() {
                return Ok(());
            }
        }
    }

    if let Some(token) = bearer_token(headers) {
        if state
            .api_keys
            .iter()
            .any(|key| constant_time_eq(key, token))
        {
            return Ok(());
        }
        if let Some(validator) = &state.access_validator {
            if validator.validate(token).is_valid() {
                return Ok(());
            }
        }
    }

    Err(ApiError(Error::Unauthorized(
        "missing or invalid API key".to_string(),
    )))
}

/// Compare two secrets without returning early at the first differing byte,
/// so response timing does not reveal how much of a guessed key matched.
/// Only the length can leak, and credential lengths are not secret.
pub(crate) fn constant_time_eq(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }
    let difference = left
        .iter()
        .zip(right)
        .fold(0u8, |difference, (left, right)| difference | (left ^ right));
    std::hint::black_box(difference) == 0
}

/// Extract the bearer token from the `Authorization` header.
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::net::SocketAddr;
    use std::sync::Arc;

    use axum::http::HeaderValue;
    use milim_core::config::ServerConfiguration;
    use milim_inference::unavailable::UnavailableBackend;

    fn state() -> AppState {
        AppState::new(
            Arc::new(UnavailableBackend::new()),
            ServerConfiguration::default(),
        )
    }

    fn headers(token: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(token) = token {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
            );
        }
        headers
    }

    #[test]
    fn constant_time_eq_matches_only_identical_secrets() {
        assert!(constant_time_eq("mobile-abc", "mobile-abc"));
        assert!(constant_time_eq("", ""));
        assert!(!constant_time_eq("mobile-abc", "mobile-abd"));
        assert!(!constant_time_eq("mobile-abc", "mobile-ab"));
        assert!(!constant_time_eq("mobile-abc", ""));
    }

    #[test]
    fn allows_requests_when_auth_is_not_configured() {
        assert!(authorize(&state(), &headers(None), None).is_ok());
    }

    #[test]
    fn accepts_static_bearer_key() {
        let state = state()
            .with_api_keys(["secret".to_string()])
            .with_loopback_trust(false);

        assert!(authorize(&state, &headers(Some("secret")), None).is_ok());
        assert!(authorize(&state, &headers(Some("wrong")), None).is_err());
    }

    #[test]
    fn loopback_trust_bypasses_token_when_enabled() {
        let state = state().with_api_keys(["secret".to_string()]);
        let peer: SocketAddr = "127.0.0.1:12345".parse().unwrap();

        assert!(authorize(&state, &headers(None), Some(peer)).is_ok());
    }

    #[test]
    fn rejects_missing_token_when_auth_is_configured() {
        let state = state()
            .with_api_keys(["secret".to_string()])
            .with_loopback_trust(false);

        assert!(authorize(&state, &headers(None), None).is_err());
    }
}
