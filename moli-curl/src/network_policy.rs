use std::net::IpAddr;

use anyhow::{Result, bail};
use cidr::AnyIpCidr;

/// Admission policy for the concrete addresses a network transport may use.
///
/// Callers must apply this policy to every address returned by DNS before
/// installing that same, complete answer on the transport. Filtering a DNS
/// answer is deliberately not supported because doing so changes address
/// selection and fallback semantics.
#[derive(Debug, Clone, Default)]
pub struct NetworkAddressPolicy {
    block_private_networks: bool,
    block_cidrs: Vec<AnyIpCidr>,
}

impl NetworkAddressPolicy {
    pub fn new(block_private_networks: bool, block_cidrs: Vec<AnyIpCidr>) -> Self {
        Self {
            block_private_networks,
            block_cidrs,
        }
    }

    pub fn is_enforced(&self) -> bool {
        self.block_private_networks || !self.block_cidrs.is_empty()
    }

    pub fn check_addresses(&self, addresses: &[IpAddr], target: &str) -> Result<()> {
        for &address in addresses {
            self.check_address(address, target)?;
        }
        Ok(())
    }

    pub fn check_address(&self, address: IpAddr, target: &str) -> Result<()> {
        if self.block_private_networks && is_private_or_internal_ip(address) {
            bail!("blocked private network address `{address}` for `{target}`");
        }
        let mapped_ipv4 = match address {
            IpAddr::V6(address) => address.to_ipv4_mapped().map(IpAddr::V4),
            IpAddr::V4(_) => None,
        };
        if let Some(cidr) = self.block_cidrs.iter().find(|cidr| {
            cidr.contains(&address)
                || mapped_ipv4
                    .as_ref()
                    .is_some_and(|mapped| cidr.contains(mapped))
        }) {
            bail!("blocked address `{address}` for `{target}` because it matches `{cidr}`");
        }
        Ok(())
    }
}

fn is_private_or_internal_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            ipv4.is_private()
                || ipv4.is_loopback()
                || ipv4.is_link_local()
                || ipv4.is_broadcast()
                || ipv4.is_documentation()
                || ipv4.is_unspecified()
                || ipv4.is_multicast()
                || matches!(ipv4.octets(), [100, second, ..] if (64..=127).contains(&second))
                || matches!(ipv4.octets(), [198, 18 | 19, ..])
                || matches!(ipv4.octets(), [240..=255, ..])
        }
        IpAddr::V6(ipv6) => {
            if let Some(ipv4) = ipv6.to_ipv4_mapped() {
                return is_private_or_internal_ip(IpAddr::V4(ipv4));
            }
            ipv6.is_loopback()
                || ipv6.is_unspecified()
                || ipv6.is_multicast()
                || ipv6.is_unicast_link_local()
                || ipv6.is_unique_local()
                || (ipv6.segments()[0] == 0x2001 && ipv6.segments()[1] == 0x0db8)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_policy_rejects_ipv4_mapped_loopback() {
        let policy = NetworkAddressPolicy::new(true, Vec::new());

        let error = policy
            .check_address("::ffff:127.0.0.1".parse().unwrap(), "wss://example.test/")
            .expect_err("mapped loopback must be private");

        assert!(
            error
                .to_string()
                .contains("blocked private network address")
        );
    }

    #[test]
    fn every_address_must_pass_policy() {
        let policy = NetworkAddressPolicy::new(true, Vec::new());

        let error = policy
            .check_addresses(
                &[
                    "93.184.216.34".parse().unwrap(),
                    "127.0.0.1".parse().unwrap(),
                ],
                "https://example.test/",
            )
            .expect_err("one private answer must reject the complete DNS result");

        assert!(error.to_string().contains("127.0.0.1"));
    }

    #[test]
    fn ipv4_cidr_rejects_an_ipv4_mapped_ipv6_address() {
        let policy = NetworkAddressPolicy::new(
            false,
            vec!["198.18.0.0/15".parse().expect("test CIDR should parse")],
        );

        let error = policy
            .check_address(
                "::ffff:198.18.0.1".parse().unwrap(),
                "https://example.test/",
            )
            .expect_err("mapped IPv4 must be checked against IPv4 CIDRs");

        assert!(error.to_string().contains("198.18.0.0/15"));
    }
}
