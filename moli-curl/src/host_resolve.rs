use std::net::IpAddr;

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostResolveOverrides {
    entries: Vec<HostResolveEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostResolveEntry {
    host: String,
    port: u16,
    addresses: Vec<IpAddr>,
}

impl HostResolveOverrides {
    /// Parse only permanent address overrides owned by Moli.
    ///
    /// libcurl also accepts `+` temporary cache entries and `-` removals. Moli
    /// deliberately excludes both: expiry or removal could return a direct
    /// request hostname to libcurl's resolver after the routing and address
    /// admission decision was made.
    pub fn parse(entries: &[String]) -> Result<Self> {
        entries
            .iter()
            .map(|entry| parse_entry(entry))
            .collect::<Result<Vec<_>>>()
            .map(|entries| Self { entries })
    }

    pub fn normalized_entries(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(HostResolveEntry::curl_entry)
            .collect()
    }

    pub fn addresses_for(&self, host: &str, port: u16) -> Option<&[IpAddr]> {
        let mut exact_match = None;
        let mut wildcard_match = None;
        for entry in &self.entries {
            if entry.port != port {
                continue;
            }
            if entry.host == "*" {
                wildcard_match = Some(entry.addresses.as_slice());
            } else if entry.host.eq_ignore_ascii_case(host) {
                exact_match = Some(entry.addresses.as_slice());
            }
        }
        exact_match.or(wildcard_match)
    }
}

pub fn validate_http_host_resolve_entries(entries: &[String]) -> Result<()> {
    HostResolveOverrides::parse(entries).map(|_| ())
}

impl HostResolveEntry {
    fn curl_entry(&self) -> String {
        let addresses = self
            .addresses
            .iter()
            .map(format_ip)
            .collect::<Vec<_>>()
            .join(",");
        format!("{}:{}:{addresses}", format_host(&self.host), self.port)
    }
}

fn parse_entry(entry: &str) -> Result<HostResolveEntry> {
    let entry = entry.trim();
    if entry.starts_with('+') || entry.starts_with('-') {
        bail!("--http-host-resolve does not support `+` or `-` prefixes; use HOST:PORT:ADDR");
    }

    let (host, port, address) = split_entry(entry)?;
    let port = port
        .parse::<u16>()
        .with_context(|| format!("invalid --http-host-resolve port in `{entry}`"))?;
    let mut addresses = Vec::new();
    for address in address.split(',').map(str::trim) {
        if address.is_empty() {
            bail!("--http-host-resolve entries must not contain empty addresses");
        }
        let address = address.trim_matches(['[', ']']);
        let address = address
            .parse::<IpAddr>()
            .with_context(|| format!("invalid --http-host-resolve address in `{entry}`"))?;
        if !addresses.contains(&address) {
            addresses.push(address);
        }
    }

    Ok(HostResolveEntry {
        host: host.trim_matches(['[', ']']).to_owned(),
        port,
        addresses,
    })
}

fn split_entry(entry: &str) -> Result<(&str, &str, &str)> {
    if let Some(entry) = entry.strip_prefix('[') {
        let Some(host_end) = entry.find(']') else {
            bail!("--http-host-resolve must be in HOST:PORT:ADDR form");
        };
        let host = &entry[..host_end];
        let remainder = &entry[host_end + 1..];
        let Some(remainder) = remainder.strip_prefix(':') else {
            bail!("--http-host-resolve must be in HOST:PORT:ADDR form");
        };
        let Some((port, address)) = remainder.split_once(':') else {
            bail!("--http-host-resolve must be in HOST:PORT:ADDR form");
        };
        if host.trim().is_empty() || port.trim().is_empty() || address.trim().is_empty() {
            bail!("--http-host-resolve must be in HOST:PORT:ADDR form");
        }
        return Ok((host.trim(), port.trim(), address.trim()));
    }

    let mut parts = entry.splitn(3, ':');
    let host = parts.next().unwrap_or_default().trim();
    let port = parts.next().unwrap_or_default().trim();
    let address = parts.next().unwrap_or_default().trim();
    if host.is_empty() || port.is_empty() || address.is_empty() {
        bail!("--http-host-resolve must be in HOST:PORT:ADDR form");
    }
    Ok((host, port, address))
}

fn format_host(host: &str) -> String {
    if host != "*" && host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}

fn format_ip(ip: &IpAddr) -> String {
    match ip {
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_normalized_and_exact_matches_beat_wildcards() {
        let overrides = HostResolveOverrides::parse(&[
            " *:443:192.0.2.1 ".to_owned(),
            " example.test:443: 198.51.100.1, [2001:db8::1] ".to_owned(),
            " [2001:db8::2]:80: 203.0.113.1 ".to_owned(),
        ])
        .unwrap();

        assert_eq!(
            overrides.normalized_entries(),
            [
                "*:443:192.0.2.1",
                "example.test:443:198.51.100.1,[2001:db8::1]",
                "[2001:db8::2]:80:203.0.113.1",
            ]
        );
        assert_eq!(
            overrides.addresses_for("EXAMPLE.TEST", 443).unwrap(),
            [
                "198.51.100.1".parse::<IpAddr>().unwrap(),
                "2001:db8::1".parse::<IpAddr>().unwrap(),
            ]
        );
        assert_eq!(
            overrides.addresses_for("other.test", 443).unwrap(),
            ["192.0.2.1".parse::<IpAddr>().unwrap()]
        );
    }

    #[test]
    fn curl_prefix_syntax_is_rejected() {
        for entry in [
            "+example.test:443:93.184.216.34",
            "-example.test:443",
            "-example.test:443:93.184.216.34",
        ] {
            let error = HostResolveOverrides::parse(&[entry.to_owned()]).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("does not support `+` or `-` prefixes"),
                "{error:#}"
            );
        }
    }
}
