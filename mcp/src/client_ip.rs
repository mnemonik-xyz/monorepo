//! Real client IP behind a reverse proxy.
//!
//! Production runs behind Caddy (or nginx). The TCP peer of every request is
//! then the proxy, not the user. A rate limit keyed on the peer IP puts every
//! user into one bucket. This module resolves the real client IP:
//!
//! - The TCP peer is not a trusted proxy: use the peer IP. Ignore
//!   `X-Forwarded-For` and `X-Real-IP`, because the client wrote them.
//! - The TCP peer is a trusted proxy: walk `X-Forwarded-For` from the right
//!   and take the first hop that is not a trusted proxy. Each proxy appends
//!   the address it saw, so the rightmost untrusted hop is the address the
//!   outermost trusted proxy saw. The leftmost hop is client-supplied and is
//!   never used. With no usable `X-Forwarded-For`, use `X-Real-IP`, then the
//!   peer.
//!
//! `TRUSTED_PROXIES` (comma-separated CIDR list) sets the trusted proxies.
//! The default is loopback plus the private ranges that Docker networks use.
//!
//! The same resolution feeds the `tower_governor` key extractor
//! ([`ClientIpKeyExtractor`]) and the per-IP free anchor counter, so one
//! client gets one bucket everywhere. [`rate_limit_key`] groups an IPv6
//! address by its /64 prefix: one host usually owns a whole /64, so a
//! per-address key would let it rotate addresses without limit.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use axum::http::{HeaderMap, Request};

/// Default `TRUSTED_PROXIES`: loopback, RFC 1918 private IPv4 (Docker bridge
/// networks use 172.16.0.0/12 and 192.168.0.0/16), IPv6 unique local.
pub const DEFAULT_TRUSTED_PROXIES: &str =
    "127.0.0.0/8,::1/128,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16,fc00::/7";

/// One CIDR block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cidr {
    network: IpAddr,
    prefix: u8,
}

impl Cidr {
    fn parse(text: &str) -> anyhow::Result<Self> {
        let text = text.trim();
        let (addr, prefix) = match text.split_once('/') {
            Some((addr, prefix)) => (addr, Some(prefix)),
            None => (text, None),
        };
        let network: IpAddr = addr
            .trim()
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid proxy address {addr:?}: {e}"))?;
        let max = if network.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) => p
                .trim()
                .parse::<u8>()
                .map_err(|e| anyhow::anyhow!("invalid prefix in {text:?}: {e}"))?,
            None => max,
        };
        if prefix > max {
            anyhow::bail!("prefix /{prefix} too long in {text:?}");
        }
        Ok(Self { network, prefix })
    }

    fn contains(&self, ip: IpAddr) -> bool {
        match (self.network, canonical(ip)) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = mask_u32(self.prefix);
                u32::from(net) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = mask_u128(self.prefix);
                u128::from(net) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

fn mask_u32(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    }
}

fn mask_u128(prefix: u8) -> u128 {
    if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - u32::from(prefix))
    }
}

/// An IPv4-mapped IPv6 address (`::ffff:1.2.3.4`) becomes plain IPv4.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        v4 => v4,
    }
}

/// The set of reverse proxies whose forwarding headers are trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedProxies {
    blocks: Vec<Cidr>,
}

impl Default for TrustedProxies {
    fn default() -> Self {
        Self::parse(DEFAULT_TRUSTED_PROXIES).unwrap_or_else(|_| Self::none())
    }
}

impl TrustedProxies {
    /// Parse a comma-separated CIDR list. A bare address means a single host.
    /// An empty string trusts no proxy (the peer IP is always used).
    pub fn parse(list: &str) -> anyhow::Result<Self> {
        let blocks = list
            .split(',')
            .filter(|item| !item.trim().is_empty())
            .map(Cidr::parse)
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self { blocks })
    }

    /// Trust no proxy: forwarding headers are always ignored.
    pub fn none() -> Self {
        Self { blocks: Vec::new() }
    }

    /// True when `ip` is inside one of the trusted blocks.
    pub fn is_trusted(&self, ip: IpAddr) -> bool {
        self.blocks.iter().any(|block| block.contains(ip))
    }

    /// The real client IP for a request from TCP peer `peer` with `headers`.
    pub fn client_ip(&self, peer: IpAddr, headers: &HeaderMap) -> IpAddr {
        let peer = canonical(peer);
        if !self.is_trusted(peer) {
            return peer;
        }
        // Every X-Forwarded-For header, in order, as one list of hops.
        let hops: Vec<&str> = headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(','))
            .map(str::trim)
            .filter(|hop| !hop.is_empty())
            .collect();
        if !hops.is_empty() {
            let mut last_parsed = None;
            for hop in hops.iter().rev() {
                match parse_hop(hop) {
                    Some(ip) if self.is_trusted(ip) => last_parsed = Some(ip),
                    Some(ip) => return ip,
                    // An unparsable hop breaks the chain: nothing left of it
                    // can be trusted. Use the last trusted hop we saw.
                    None => return last_parsed.unwrap_or(peer),
                }
            }
            // Every hop is a trusted proxy: the leftmost one is the client.
            return last_parsed.unwrap_or(peer);
        }
        if let Some(ip) = headers
            .get("x-real-ip")
            .and_then(|value| value.to_str().ok())
            .and_then(parse_hop)
        {
            return ip;
        }
        peer
    }
}

/// Parse one forwarding hop: `1.2.3.4`, `1.2.3.4:5678`, `2001:db8::1` or
/// `[2001:db8::1]:443`.
fn parse_hop(hop: &str) -> Option<IpAddr> {
    let hop = hop.trim().trim_matches('"');
    if let Ok(ip) = hop.parse::<IpAddr>() {
        return Some(canonical(ip));
    }
    if let Ok(addr) = hop.parse::<SocketAddr>() {
        return Some(canonical(addr.ip()));
    }
    None
}

/// Rate-limit key for `ip`: the IPv4 address itself, or the /64 prefix of an
/// IPv6 address.
pub fn rate_limit_key(ip: IpAddr) -> IpAddr {
    match canonical(ip) {
        IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from(u128::from(v6) & mask_u128(64))),
        v4 => v4,
    }
}

/// The TCP peer from axum's `ConnectInfo`, when the server was started with
/// `into_make_service_with_connect_info`.
pub fn peer_ip<B>(req: &Request<B>) -> Option<IpAddr> {
    req.extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip())
}

/// Resolve the client for a request: the real client IP, or `None` when the
/// TCP peer is unknown (in-process tests only; production always has
/// `ConnectInfo`).
pub fn resolve<B>(proxies: &TrustedProxies, req: &Request<B>) -> Option<IpAddr> {
    peer_ip(req).map(|peer| proxies.client_ip(peer, req.headers()))
}

/// `tower_governor` key extractor: the rate-limit key of the real client IP.
#[derive(Debug, Clone)]
pub struct ClientIpKeyExtractor {
    proxies: Arc<TrustedProxies>,
}

impl ClientIpKeyExtractor {
    pub fn new(proxies: Arc<TrustedProxies>) -> Self {
        Self { proxies }
    }
}

impl tower_governor::key_extractor::KeyExtractor for ClientIpKeyExtractor {
    type Key = IpAddr;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, tower_governor::GovernorError> {
        resolve(&self.proxies, req)
            .map(rate_limit_key)
            .ok_or(tower_governor::GovernorError::UnableToExtractKey)
    }
}

/// Handler argument: the real client IP of the request, or `None` when the
/// TCP peer is unknown. Its `FromRequestParts` impl (in `api.rs`) reads the
/// trusted proxies from `McpState` and never rejects.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClientIp(pub Option<IpAddr>);

/// Unspecified IPv4 address, the bucket for a request with no known peer.
pub const UNKNOWN_CLIENT: IpAddr = IpAddr::V4(Ipv4Addr::UNSPECIFIED);

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn untrusted_peer_spoofing_x_forwarded_for_is_ignored() {
        let proxies = TrustedProxies::default();
        let h = headers(&[("x-forwarded-for", "1.1.1.1"), ("x-real-ip", "2.2.2.2")]);
        // A public peer writes its own forwarding headers: ignored.
        assert_eq!(proxies.client_ip(ip("203.0.113.7"), &h), ip("203.0.113.7"));
    }

    #[test]
    fn trusted_proxy_uses_the_rightmost_untrusted_hop() {
        let proxies = TrustedProxies::default();
        // The client sent a forged "6.6.6.6"; Caddy appended the real peer.
        let h = headers(&[("x-forwarded-for", "6.6.6.6, 198.51.100.4")]);
        assert_eq!(proxies.client_ip(ip("172.18.0.2"), &h), ip("198.51.100.4"));
        // Two proxies: the inner one (10.0.0.3) is skipped as trusted.
        let h = headers(&[("x-forwarded-for", "6.6.6.6, 198.51.100.4, 10.0.0.3")]);
        assert_eq!(proxies.client_ip(ip("127.0.0.1"), &h), ip("198.51.100.4"));
        // Repeated headers are one list.
        let mut h = headers(&[("x-forwarded-for", "6.6.6.6")]);
        h.append("x-forwarded-for", HeaderValue::from_static("198.51.100.9"));
        assert_eq!(proxies.client_ip(ip("127.0.0.1"), &h), ip("198.51.100.9"));
    }

    #[test]
    fn trusted_proxy_falls_back_to_x_real_ip_then_peer() {
        let proxies = TrustedProxies::default();
        let h = headers(&[("x-real-ip", "198.51.100.5")]);
        assert_eq!(proxies.client_ip(ip("127.0.0.1"), &h), ip("198.51.100.5"));
        assert_eq!(
            proxies.client_ip(ip("127.0.0.1"), &HeaderMap::new()),
            ip("127.0.0.1")
        );
    }

    #[test]
    fn garbage_hop_stops_the_walk() {
        let proxies = TrustedProxies::default();
        let h = headers(&[("x-forwarded-for", "198.51.100.1, not-an-ip, 10.0.0.9")]);
        // 10.0.0.9 is trusted, "not-an-ip" breaks the chain: use 10.0.0.9.
        assert_eq!(proxies.client_ip(ip("127.0.0.1"), &h), ip("10.0.0.9"));
        let h = headers(&[("x-forwarded-for", "[2001:db8::5]:443")]);
        assert_eq!(proxies.client_ip(ip("::1"), &h), ip("2001:db8::5"));
    }

    #[test]
    fn empty_list_trusts_nobody_and_parse_rejects_garbage() {
        let proxies = TrustedProxies::parse("").unwrap();
        let h = headers(&[("x-forwarded-for", "198.51.100.1")]);
        assert_eq!(proxies.client_ip(ip("127.0.0.1"), &h), ip("127.0.0.1"));
        assert!(TrustedProxies::parse("10.0.0.0/33").is_err());
        assert!(TrustedProxies::parse("nonsense").is_err());
        let custom = TrustedProxies::parse("203.0.113.10, 2001:db8::/32").unwrap();
        assert!(custom.is_trusted(ip("203.0.113.10")));
        assert!(!custom.is_trusted(ip("203.0.113.11")));
        assert!(custom.is_trusted(ip("2001:db8:1::1")));
        // IPv4-mapped IPv6 matches the IPv4 block.
        assert!(TrustedProxies::default().is_trusted(ip("::ffff:10.1.2.3")));
    }

    #[test]
    fn ipv6_rate_limit_key_is_the_64_prefix() {
        assert_eq!(
            rate_limit_key(ip("2001:db8:1:2:aaaa:bbbb:cccc:dddd")),
            ip("2001:db8:1:2::")
        );
        assert_eq!(rate_limit_key(ip("198.51.100.1")), ip("198.51.100.1"));
        assert_eq!(
            rate_limit_key(ip("::ffff:198.51.100.1")),
            ip("198.51.100.1")
        );
    }

    #[test]
    fn key_extractor_uses_connect_info_and_rejects_a_missing_peer() {
        use tower_governor::key_extractor::KeyExtractor;
        let extractor = ClientIpKeyExtractor::new(Arc::new(TrustedProxies::default()));
        let mut req = Request::builder()
            .header("x-forwarded-for", "6.6.6.6")
            .body(())
            .unwrap();
        assert!(extractor.extract(&req).is_err());
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(SocketAddr::from((
                [203, 0, 113, 50],
                4000,
            ))));
        assert_eq!(extractor.extract(&req).unwrap(), ip("203.0.113.50"));
    }
}
