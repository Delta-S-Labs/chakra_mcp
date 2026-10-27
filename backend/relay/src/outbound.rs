//! Requests the relay makes to URLs an agent supplied: fetching push
//! agents' Agent Cards and forwarding calls to them.
//!
//! An agent controls those URLs (its `agent_card_url`, and the interface URL
//! inside the card it serves), so without a check it could point the relay
//! at the relay's own network: loopback, the Docker or cluster network, or
//! the cloud metadata address. Every such request goes through a client from
//! [`client_builder`], which:
//!
//! - resolves host names itself and connects only to addresses the policy
//!   allows, so a name can't resolve to one address when checked and
//!   another when connected (DNS rebinding);
//! - checks a redirect's target the same way, and follows at most 3.
//!
//! A URL whose host is an IP address skips DNS altogether, so callers also
//! run [`check_url`] before sending.
//!
//! Which addresses are allowed: public ones always. Loopback and private
//! ranges only when `OUTBOUND_ALLOW_PRIVATE` is on, which is the default on
//! a self-hosted server (agents there often run on the LAN) and off on a
//! managed one. Link-local, cloud metadata, multicast and reserved addresses
//! never.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use chakramcp_shared::hosting::{env_value, parse_bool, HostingMode};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use url::{Host, Url};

/// Redirects followed before giving up.
const MAX_REDIRECTS: usize = 3;

/// Which addresses outbound requests may reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutboundPolicy {
    /// Loopback and private ranges (RFC 1918, carrier-grade NAT, IPv6
    /// unique local) are allowed too.
    pub allow_private: bool,
}

impl Default for OutboundPolicy {
    fn default() -> Self {
        Self::for_mode(HostingMode::default())
    }
}

impl OutboundPolicy {
    /// The hosting mode's default: private addresses allowed on a
    /// self-hosted server, not on a managed one.
    pub fn for_mode(mode: HostingMode) -> Self {
        Self {
            allow_private: mode == HostingMode::SelfHosted,
        }
    }

    /// `OUTBOUND_ALLOW_PRIVATE` when set, else the mode's default. A value
    /// that isn't a boolean stops startup.
    pub fn from_sources(
        env: impl Fn(&str) -> Option<String>,
        mode: HostingMode,
    ) -> anyhow::Result<Self> {
        Ok(match env_value(&env, "OUTBOUND_ALLOW_PRIVATE") {
            Some(raw) => Self {
                allow_private: parse_bool("OUTBOUND_ALLOW_PRIVATE", &raw)?,
            },
            None => Self::for_mode(mode),
        })
    }

    /// From the environment, for the standalone relay.
    pub fn from_env(mode: HostingMode) -> anyhow::Result<Self> {
        Self::from_sources(|key| std::env::var(key).ok(), mode)
    }

    /// The startup log line.
    pub fn summary(&self) -> &'static str {
        if self.allow_private {
            "outbound requests to agents: public and private addresses"
        } else {
            "outbound requests to agents: public addresses only"
        }
    }

    /// Whether a request may connect to `ip`.
    pub fn allows(&self, ip: IpAddr) -> bool {
        match classify(ip) {
            Class::Public => true,
            Class::Private => self.allow_private,
            Class::Never => false,
        }
    }
}

/// Why a URL was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutboundError {
    #[error("not an absolute http(s) URL with a host")]
    BadUrl,
    #[error("{0} is a private or reserved address, which this server doesn't call")]
    Blocked(String),
}

/// Check a URL before sending to it: http(s), a host, and, when the host is
/// an IP address (which skips the resolver), an address the policy allows.
/// Host names are checked when they resolve.
pub fn check_url(raw: &str, policy: OutboundPolicy) -> Result<Url, OutboundError> {
    let url = Url::parse(raw).map_err(|_| OutboundError::BadUrl)?;
    check_parsed(&url, policy)?;
    Ok(url)
}

fn check_parsed(url: &Url, policy: OutboundPolicy) -> Result<(), OutboundError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(OutboundError::BadUrl);
    }
    let ip = match url.host() {
        Some(Host::Ipv4(ip)) => IpAddr::V4(ip),
        Some(Host::Ipv6(ip)) => IpAddr::V6(ip),
        // `localhost` is loopback whatever DNS says (RFC 6761), and an
        // obvious mistake worth a clear error before any lookup.
        Some(Host::Domain(name)) if is_localhost(name) => IpAddr::V4(Ipv4Addr::LOCALHOST),
        Some(Host::Domain(_)) => return Ok(()),
        None => return Err(OutboundError::BadUrl),
    };
    if policy.allows(ip) {
        Ok(())
    } else {
        Err(OutboundError::Blocked(
            url.host_str().unwrap_or_default().to_owned(),
        ))
    }
}

fn is_localhost(name: &str) -> bool {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    name == "localhost" || name.ends_with(".localhost")
}

/// A client builder that resolves and redirects under `policy`. Add
/// timeouts and the like, then `build()`.
pub fn client_builder(policy: OutboundPolicy) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .dns_resolver(Arc::new(GuardedResolver { policy }))
        // A proxy would resolve names itself, out of the resolver's reach.
        .no_proxy()
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else if let Err(e) = check_parsed(attempt.url(), policy) {
                attempt.error(e)
            } else {
                attempt.follow()
            }
        }))
}

/// Resolves with the system resolver and keeps only allowed addresses.
struct GuardedResolver {
    policy: OutboundPolicy,
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let policy = self.policy;
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let allowed: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|addr| policy.allows(addr.ip()))
                .collect();
            if allowed.is_empty() {
                return Err(blocked(host));
            }
            Ok(Box::new(allowed.into_iter()) as Addrs)
        })
    }
}

fn blocked(host: String) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(OutboundError::Blocked(host))
}

/// Where an address sits for [`OutboundPolicy::allows`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Public,
    /// Loopback and private networks: allowed when the policy says so.
    Private,
    /// Never: link-local (cloud metadata lives there), metadata addresses
    /// outside it, multicast, documentation and reserved ranges.
    Never,
}

fn classify(ip: IpAddr) -> Class {
    match ip {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => classify_v6(v6),
    }
}

fn classify_v4(ip: Ipv4Addr) -> Class {
    let [a, b, c, _] = ip.octets();
    // Metadata services at addresses outside link-local: Alibaba Cloud's
    // (inside carrier-grade NAT) and Azure's wire server (a public address).
    if ip == Ipv4Addr::new(100, 100, 100, 200) || ip == Ipv4Addr::new(168, 63, 129, 16) {
        return Class::Never;
    }
    if a == 0                                   // "this network", incl. 0.0.0.0
        || ip.is_link_local()                   // 169.254/16
        || ip.is_multicast()                    // 224/4
        || ip.is_broadcast()
        || a >= 240                             // reserved
        || (a == 192 && b == 0 && c == 0)       // IETF protocol assignments
        || ip.is_documentation()
    {
        return Class::Never;
    }
    if ip.is_loopback()
        || ip.is_private()                      // 10/8, 172.16/12, 192.168/16
        || (a == 100 && (64..128).contains(&b)) // carrier-grade NAT, 100.64/10
        || (a == 198 && (b == 18 || b == 19))
    // benchmarking, 198.18/15
    {
        return Class::Private;
    }
    Class::Public
}

fn classify_v6(ip: Ipv6Addr) -> Class {
    // An IPv4 address inside IPv6: judge the IPv4 address.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return classify_v4(v4);
    }
    let seg = ip.segments();
    // NAT64 (64:ff9b::/96) carries the IPv4 address in its last 32 bits.
    if seg[0] == 0x64 && seg[1] == 0xff9b && seg[2..6] == [0, 0, 0, 0] {
        return classify_v4(Ipv4Addr::new(
            (seg[6] >> 8) as u8,
            seg[6] as u8,
            (seg[7] >> 8) as u8,
            seg[7] as u8,
        ));
    }
    // 6to4 (2002::/16) carries it in bits 16–48.
    if seg[0] == 0x2002 {
        return classify_v4(Ipv4Addr::new(
            (seg[1] >> 8) as u8,
            seg[1] as u8,
            (seg[2] >> 8) as u8,
            seg[2] as u8,
        ));
    }
    // AWS's IPv6 metadata address, inside unique local.
    if ip == Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254) {
        return Class::Never;
    }
    if ip.is_unspecified()
        || ip.is_multicast()
        || (seg[0] & 0xffc0) == 0xfe80              // link-local, fe80::/10
        || (seg[0] == 0x2001 && seg[1] == 0x0db8)   // documentation
        || (seg[0] == 0x2001 && seg[1] == 0)        // Teredo, hides an IPv4 address
        || (seg[0] == 0x0100 && seg[1..4] == [0, 0, 0]) // discard, 100::/64
        || seg[..6] == [0, 0, 0, 0, 0, 0] && !ip.is_loopback()
    // IPv4-compatible
    {
        return Class::Never;
    }
    // Unique local (fc00::/7) and the deprecated site-local (fec0::/10).
    if ip.is_loopback() || (seg[0] & 0xfe00) == 0xfc00 || (seg[0] & 0xffc0) == 0xfec0 {
        return Class::Private;
    }
    Class::Public
}

#[cfg(test)]
mod tests {
    use super::*;

    const DENY: OutboundPolicy = OutboundPolicy {
        allow_private: false,
    };
    const ALLOW: OutboundPolicy = OutboundPolicy {
        allow_private: true,
    };

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn public_addresses_pass_either_way() {
        for s in ["1.1.1.1", "93.184.216.34", "2606:4700:4700::1111"] {
            assert!(DENY.allows(ip(s)), "{s}");
            assert!(ALLOW.allows(ip(s)), "{s}");
        }
    }

    #[test]
    fn private_addresses_need_the_setting() {
        for s in [
            "127.0.0.1",
            "127.8.9.10",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "100.64.0.1",
            "198.18.0.1",
            "::1",
            "fd12:3456::1",
            "::ffff:10.0.0.1",
            "64:ff9b::a00:1",   // NAT64 of 10.0.0.1
            "2002:0a00:0001::", // 6to4 of 10.0.0.1
        ] {
            assert!(!DENY.allows(ip(s)), "{s} should be blocked by default");
            assert!(ALLOW.allows(ip(s)), "{s} should pass with private allowed");
        }
    }

    #[test]
    fn metadata_link_local_and_reserved_never_pass() {
        for s in [
            "169.254.169.254",
            "169.254.0.1",
            "100.100.100.200",
            "168.63.129.16",
            "0.0.0.0",
            "0.1.2.3",
            "224.0.0.1",
            "255.255.255.255",
            "240.0.0.1",
            "192.0.0.1",
            "192.0.2.1",
            "fe80::1",
            "fd00:ec2::254",
            "::",
            "ff02::1",
            "2001:db8::1",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
            "::ffff:169.254.169.254",
            "64:ff9b::a9fe:a9fe", // NAT64 of 169.254.169.254
            "::a9fe:a9fe",        // IPv4-compatible 169.254.169.254
        ] {
            assert!(!DENY.allows(ip(s)), "{s}");
            assert!(!ALLOW.allows(ip(s)), "{s} must stay blocked");
        }
    }

    #[test]
    fn urls_with_ip_hosts_are_checked_before_sending() {
        assert!(check_url("https://example.com/card.json", DENY).is_ok());
        assert!(check_url("http://93.184.216.34/", DENY).is_ok());
        for raw in [
            "http://127.0.0.1:8080/",
            "http://[::1]/",
            "http://2130706433/", // 127.0.0.1 as one number
            "http://0x7f.1/",
            "http://localhost:8090/healthz",
            "http://api.localhost/",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::ffff:169.254.169.254]/",
        ] {
            assert!(
                matches!(check_url(raw, DENY), Err(OutboundError::Blocked(_))),
                "{raw}"
            );
        }
        assert!(check_url("http://127.0.0.1:8080/", ALLOW).is_ok());
        assert!(check_url("http://localhost/", ALLOW).is_ok());
        assert!(matches!(
            check_url("http://169.254.169.254/", ALLOW),
            Err(OutboundError::Blocked(_))
        ));
        assert_eq!(
            check_url("ftp://example.com/", DENY),
            Err(OutboundError::BadUrl)
        );
        assert_eq!(check_url("not a url", DENY), Err(OutboundError::BadUrl));
    }

    fn policy(value: Option<&str>, mode: HostingMode) -> anyhow::Result<OutboundPolicy> {
        let value = value.map(str::to_owned);
        OutboundPolicy::from_sources(
            move |key| {
                (key == "OUTBOUND_ALLOW_PRIVATE")
                    .then(|| value.clone())
                    .flatten()
            },
            mode,
        )
    }

    #[test]
    fn the_default_follows_the_hosting_mode_and_the_setting_overrides_it() {
        let allow = |value, mode| policy(value, mode).unwrap().allow_private;
        assert!(allow(None, HostingMode::SelfHosted));
        assert!(!allow(None, HostingMode::Managed));
        assert!(!allow(Some("false"), HostingMode::SelfHosted));
        assert!(allow(Some("true"), HostingMode::Managed));
        // Empty counts as unset (Compose passes one for every key left out).
        assert!(!allow(Some(""), HostingMode::Managed));
        assert!(policy(Some("maybe"), HostingMode::Managed).is_err());
    }

    /// A one-shot HTTP server on loopback that answers `200 ok`.
    async fn local_server() -> SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok",
                    )
                    .await;
            }
        });
        addr
    }

    /// Through the resolver: `localhost` resolves to loopback addresses,
    /// which the filter drops.
    #[tokio::test]
    async fn names_resolving_to_blocked_addresses_never_connect() {
        let addr = local_server().await;
        let url = format!("http://localhost:{}/", addr.port());
        let denied = client_builder(DENY).build().unwrap();
        let err = denied.get(&url).send().await.unwrap_err();
        assert!(err.is_connect() || err.is_request(), "{err:?}");
        let allowed = client_builder(ALLOW).build().unwrap();
        let ok = allowed.get(&url).send().await.unwrap();
        assert_eq!(ok.text().await.unwrap(), "ok");
    }

    #[tokio::test]
    async fn redirects_to_blocked_addresses_are_refused() {
        // A public-looking first hop isn't needed: the redirect policy
        // checks every target, so a loopback server redirecting to the
        // metadata address shows it even with private addresses allowed.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 302 Found\r\nlocation: http://169.254.169.254/latest/\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                    )
                    .await;
            }
        });
        let client = client_builder(ALLOW).build().unwrap();
        let err = client
            .get(format!("http://127.0.0.1:{}/", addr.port()))
            .send()
            .await
            .unwrap_err();
        assert!(err.is_redirect(), "{err:?}");
    }
}
