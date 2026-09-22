//! DNS-rebinding protection for every embedded listener.
//!
//! A hostile web page can point its own DNS name at `127.0.0.1` (or a LAN
//! address) and then talk to milim with the browser treating the responses as
//! same-origin. The browser still sends the attacker's name in `Host`, so each
//! router rejects requests whose `Host` names something the listener was never
//! meant to serve. IP literals are always safe: a page addressed by IP cannot
//! rebind to a different server.

use std::net::{IpAddr, SocketAddr};
use std::sync::OnceLock;

use axum::extract::{Request, State};
use axum::http::uri::Authority;
use axum::http::{header::HOST, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use milim_core::api::openai::ErrorEnvelope;

/// Comma-separated extra host names accepted by every listener, for setups
/// such as a reverse proxy with its own DNS name in front of `milim serve`.
pub const ALLOWED_HOSTS_ENV: &str = "MILIM_ALLOWED_HOSTS";

/// Which `Host` names a listener accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostPolicy {
    /// Loopback listener: `localhost` and loopback IP literals such as
    /// `127.0.0.1` and `[::1]`, on any port.
    Loopback,
    /// LAN or Tailscale listener: any IP literal, `localhost`, `*.local`,
    /// `*.ts.net`, and this machine's host name.
    Network,
}

impl HostPolicy {
    /// Choose the policy for a listener from the address it is bound to.
    pub fn for_bound_address(addr: SocketAddr) -> Self {
        if addr.ip().is_loopback() {
            Self::Loopback
        } else {
            Self::Network
        }
    }

    /// Whether a raw `Host` header value (`name`, `name:port`, `[v6]:port`)
    /// is acceptable for this listener.
    pub fn allows(self, host_header: &str) -> bool {
        let Some(host) = normalized_host(host_header) else {
            return false;
        };
        if host == "localhost" || extra_allowed_hosts().iter().any(|name| *name == host) {
            return true;
        }
        if let Ok(ip) = host.parse::<IpAddr>() {
            return match self {
                Self::Loopback => ip.is_loopback(),
                Self::Network => true,
            };
        }
        match self {
            Self::Loopback => false,
            Self::Network => {
                host.ends_with(".local")
                    || host.ends_with(".ts.net")
                    || machine_host_names().iter().any(|name| *name == host)
            }
        }
    }

    /// Resolve lazily computed host names before the first request.
    pub(crate) fn warm(self) {
        let _ = extra_allowed_hosts();
        if self == Self::Network {
            let _ = machine_host_names();
        }
    }
}

/// Axum middleware that rejects requests whose `Host` (or absolute-form
/// request authority) is not allowed by the listener's [`HostPolicy`].
pub(crate) async fn validate_host(
    State(policy): State<HostPolicy>,
    request: Request,
    next: Next,
) -> Response {
    let header = request.headers().get(HOST).map(|value| value.to_str());
    let authority = request.uri().authority().map(Authority::as_str);
    let allowed = match header {
        Some(Err(_)) => false,
        Some(Ok(host)) => {
            policy.allows(host) && authority.is_none_or(|authority| policy.allows(authority))
        }
        None => authority.is_some_and(|authority| policy.allows(authority)),
    };
    if allowed {
        return next.run(request).await;
    }
    tracing::warn!(
        target: "milim_server::host_guard",
        ?policy,
        "rejected request with a disallowed Host header"
    );
    (
        StatusCode::FORBIDDEN,
        Json(ErrorEnvelope::new(
            "request Host is not allowed for this listener",
            "invalid_request_error",
        )),
    )
        .into_response()
}

/// Lower-case host without port, IPv6 brackets, or a trailing root dot.
fn normalized_host(raw: &str) -> Option<String> {
    let authority = raw.trim().parse::<Authority>().ok()?;
    if authority.as_str().contains('@') {
        return None;
    }
    let host = authority.host();
    let host = host
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host);
    let host = host.strip_suffix('.').unwrap_or(host);
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

fn normalized_names(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut names = Vec::new();
    for value in values {
        let Some(name) = normalized_host(&value) else {
            continue;
        };
        if name.parse::<IpAddr>().is_ok() {
            continue;
        }
        // Also accept the short name (`desk` for `desk.example.lan`), which is
        // how MagicDNS and NetBIOS usually address the machine.
        let short = name.split('.').next().unwrap_or_default().to_string();
        for candidate in [name, short] {
            if !candidate.is_empty() && !names.contains(&candidate) {
                names.push(candidate);
            }
        }
    }
    names
}

fn extra_allowed_hosts() -> &'static [String] {
    static HOSTS: OnceLock<Vec<String>> = OnceLock::new();
    HOSTS.get_or_init(|| {
        let raw = std::env::var(ALLOWED_HOSTS_ENV).unwrap_or_default();
        let mut hosts = Vec::new();
        for entry in raw.split(',') {
            if let Some(host) = normalized_host(entry) {
                if !hosts.contains(&host) {
                    hosts.push(host);
                }
            }
        }
        hosts
    })
}

fn machine_host_names() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut values = ["COMPUTERNAME", "HOSTNAME"]
            .into_iter()
            .filter_map(|key| std::env::var(key).ok())
            .collect::<Vec<_>>();
        #[cfg(unix)]
        {
            if let Ok(name) = std::fs::read_to_string("/etc/hostname") {
                values.push(name);
            }
            if values.iter().all(|value| value.trim().is_empty()) {
                if let Ok(output) = std::process::Command::new("hostname").output() {
                    values.push(String::from_utf8_lossy(&output.stdout).into_owned());
                }
            }
        }
        normalized_names(values)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_policy_accepts_only_loopback_names() {
        for host in [
            "localhost",
            "LOCALHOST:7377",
            "localhost.",
            "127.0.0.1",
            "127.0.0.1:7377",
            "[::1]",
            "[::1]:7377",
        ] {
            assert!(HostPolicy::Loopback.allows(host), "{host}");
        }
        for host in [
            "evil.example",
            "evil.example:7377",
            "localhost.evil.example",
            "192.168.1.20:7377",
            "desk.local",
            "desk.tailnet.ts.net",
            "user@localhost",
            "",
            "   ",
        ] {
            assert!(!HostPolicy::Loopback.allows(host), "{host}");
        }
    }

    #[test]
    fn network_policy_accepts_ip_literals_and_private_names() {
        for host in [
            "localhost:49152",
            "192.168.1.20:49152",
            "100.101.102.103",
            "[fd7a:115c:a1e0::1]:443",
            "milim-abc123.local.",
            "desk.tailnet-1234.ts.net",
            "DESK.TAILNET-1234.TS.NET:443",
        ] {
            assert!(HostPolicy::Network.allows(host), "{host}");
        }
        for host in [
            "evil.example",
            "attacker.ts.net.evil.example",
            "local",
            "ts.net",
        ] {
            assert!(!HostPolicy::Network.allows(host), "{host}");
        }
    }

    #[test]
    fn machine_names_include_short_names() {
        assert_eq!(
            normalized_names(["Desk.Example.Lan.\n".to_string(), "desk".to_string()]),
            vec!["desk.example.lan".to_string(), "desk".to_string()]
        );
        assert!(normalized_names(["10.0.0.2".to_string()]).is_empty());
    }

    #[test]
    fn policy_follows_the_bound_address() {
        assert_eq!(
            HostPolicy::for_bound_address("127.0.0.1:7377".parse().unwrap()),
            HostPolicy::Loopback
        );
        assert_eq!(
            HostPolicy::for_bound_address("[::1]:7377".parse().unwrap()),
            HostPolicy::Loopback
        );
        assert_eq!(
            HostPolicy::for_bound_address("0.0.0.0:7377".parse().unwrap()),
            HostPolicy::Network
        );
    }
}
