//! Server-side geo enrichment — country + city from IP via MaxMind GeoLite2.
//!
//! CO-178: enriches telemetry_events with country/city before the IP is hashed
//! and discarded. Raw IP is never stored.
//!
//! City granularity is "São Paulo", not a street address. GeoLite2 attribution:
//! This product includes GeoLite2 data created by MaxMind, available from
//! <https://www.maxmind.com>.

use std::net::IpAddr;
use std::str::FromStr;

pub struct GeoDb(Option<maxminddb::Reader<Vec<u8>>>);

impl GeoDb {
    pub fn open(path: &str) -> Self {
        match std::fs::read(path) {
            Ok(data) => match maxminddb::Reader::from_source(data) {
                Ok(reader) => {
                    tracing::info!("GeoIP database loaded from {path}");
                    Self(Some(reader))
                }
                Err(e) => {
                    tracing::warn!(
                        "GeoIP database parse error ({path}): {e} — geo enrichment disabled"
                    );
                    Self(None)
                }
            },
            Err(_) => {
                tracing::info!("GeoIP database not found at {path} — geo enrichment disabled");
                Self(None)
            }
        }
    }

    pub fn disabled() -> Self {
        Self(None)
    }
}

fn is_private_ip(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(ip) => {
            ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified()
        }
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unspecified(),
    }
}

/// Look up country (ISO 3166-1 alpha-2) and city name for an IP address.
///
/// Returns `(None, None)` for private/loopback IPs, unparseable input, or when
/// the database is not loaded. Never panics.
pub fn geo_lookup(db: &GeoDb, ip: &str) -> (Option<String>, Option<String>) {
    let addr = match IpAddr::from_str(ip.trim()) {
        Ok(a) => a,
        Err(_) => return (None, None),
    };

    if is_private_ip(addr) {
        return (None, None);
    }

    let reader = match &db.0 {
        Some(r) => r,
        None => return (None, None),
    };

    match reader.lookup(addr) {
        Ok(city) => {
            let city: maxminddb::geoip2::City<'_> = city;
            let country = city.country.and_then(|c| c.iso_code).map(str::to_owned);
            let city_name = city
                .city
                .and_then(|c| c.names)
                .and_then(|n| n.get("en").copied())
                .map(str::to_owned);
            (country, city_name)
        }
        Err(_) => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_returns_none() {
        let db = GeoDb::disabled();
        assert_eq!(geo_lookup(&db, "127.0.0.1"), (None, None));
    }

    #[test]
    fn private_192_returns_none() {
        let db = GeoDb::disabled();
        assert_eq!(geo_lookup(&db, "192.168.1.1"), (None, None));
    }

    #[test]
    fn private_10_returns_none() {
        let db = GeoDb::disabled();
        assert_eq!(geo_lookup(&db, "10.0.0.1"), (None, None));
    }

    #[test]
    fn private_172_returns_none() {
        let db = GeoDb::disabled();
        assert_eq!(geo_lookup(&db, "172.16.0.1"), (None, None));
    }

    #[test]
    fn invalid_ip_returns_none() {
        let db = GeoDb::disabled();
        assert_eq!(geo_lookup(&db, "not-an-ip"), (None, None));
    }

    #[test]
    fn empty_input_returns_none() {
        let db = GeoDb::disabled();
        assert_eq!(geo_lookup(&db, ""), (None, None));
    }

    #[test]
    fn no_db_public_ip_returns_none() {
        // Without a loaded database any public IP returns (None, None) gracefully.
        let db = GeoDb::disabled();
        assert_eq!(geo_lookup(&db, "8.8.8.8"), (None, None));
    }
}
