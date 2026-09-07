use std::net::IpAddr;

#[derive(Debug, PartialEq)]
pub(super) struct ParsedIceCandidate<'a> {
    pub(super) foundation: &'a str,
    pub(super) component: &'static str,
    pub(super) priority: u32,
    pub(super) address: &'a str,
    pub(super) protocol: &'static str,
    pub(super) port: u16,
    pub(super) kind: &'static str,
    pub(super) tcp_type: Option<&'static str>,
    pub(super) related_address: Option<&'a str>,
    pub(super) related_port: Option<u16>,
}

// RFC 5245 section 15.1 and the TCP extension in RFC 6544 section 4.5.
// Return no derived fields if parsing or an attribute's value is invalid;
// RTCIceCandidate itself still retains the original candidate string.
pub(super) fn parse_ice_candidate(input: &str) -> Option<ParsedIceCandidate<'_>> {
    let fields: Vec<_> = input.strip_prefix("candidate:")?.split(' ').collect();
    if fields.len() < 8 || fields.iter().any(|field| field.is_empty()) {
        return None;
    }
    let foundation = fields[0];
    if foundation.len() > 32
        || !foundation
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'+' | b'/'))
    {
        return None;
    }
    let component = match decimal(fields[1], 5)? {
        1 => "rtp",
        2 => "rtcp",
        _ => return None,
    };
    let protocol = if fields[2].eq_ignore_ascii_case("udp") {
        "udp"
    } else if fields[2].eq_ignore_ascii_case("tcp") {
        "tcp"
    } else {
        return None;
    };
    let priority = decimal(fields[3], 10)?;
    if !(1..=i32::MAX as u32).contains(&priority) || !address_is_valid(fields[4]) {
        return None;
    }
    let port = u16::try_from(decimal(fields[5], usize::MAX)?).ok()?;
    if !fields[6].eq_ignore_ascii_case("typ") {
        return None;
    }
    let kind = ["host", "srflx", "prflx", "relay"]
        .into_iter()
        .find(|kind| fields[7].eq_ignore_ascii_case(kind))?;
    let mut candidate = ParsedIceCandidate {
        foundation,
        component,
        priority,
        address: fields[4],
        protocol,
        port,
        kind,
        tcp_type: None,
        related_address: None,
        related_port: None,
    };
    let extensions = fields[8..].chunks_exact(2);
    if !extensions.remainder().is_empty() {
        return None;
    }
    for pair in extensions {
        if pair
            .iter()
            .any(|field| field.bytes().any(|ch| ch.is_ascii_control()))
        {
            return None;
        }
        if pair[0].eq_ignore_ascii_case("raddr") {
            if !address_is_valid(pair[1]) || candidate.related_address.replace(pair[1]).is_some() {
                return None;
            }
        } else if pair[0].eq_ignore_ascii_case("rport") {
            let port = u16::try_from(decimal(pair[1], usize::MAX)?).ok()?;
            if candidate.related_port.replace(port).is_some() {
                return None;
            }
        } else if pair[0].eq_ignore_ascii_case("tcptype") && protocol == "tcp" {
            let tcp_type = ["active", "passive", "so"]
                .into_iter()
                .find(|kind| pair[1].eq_ignore_ascii_case(kind))?;
            if candidate.tcp_type.replace(tcp_type).is_some() {
                return None;
            }
        }
    }
    if protocol == "tcp" && candidate.tcp_type.is_none() {
        return None;
    }
    Some(candidate)
}

fn decimal(input: &str, max_digits: usize) -> Option<u32> {
    if input.is_empty() || input.len() > max_digits || !input.bytes().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    input.parse().ok()
}

fn address_is_valid(input: &str) -> bool {
    if input.parse::<IpAddr>().is_ok() {
        return true;
    }
    let hostname = input.strip_suffix('.').unwrap_or(input);
    !hostname.is_empty()
        && hostname.len() <= 253
        && hostname.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ice_candidate_parser_handles_udp_tcp_ipv6_and_mdns() {
        let udp = parse_ice_candidate(
            "candidate:ab+/12 1 UDP 2113937151 2001:db8::1 65535 typ host generation 0 ufrag test",
        )
        .unwrap();
        assert_eq!(udp.foundation, "ab+/12");
        assert_eq!(udp.component, "rtp");
        assert_eq!(udp.protocol, "udp");
        assert_eq!(udp.priority, 2113937151);
        assert_eq!(udp.address, "2001:db8::1");
        assert_eq!(udp.port, 65535);
        assert_eq!(udp.tcp_type, None);
        let tcp = parse_ice_candidate("candidate:123 2 tcp 123456 foo.local 9 typ srflx raddr www.example.com rport 22222 tcptype active").unwrap();
        assert_eq!(tcp.component, "rtcp");
        assert_eq!(tcp.tcp_type, Some("active"));
        assert_eq!(tcp.related_address, Some("www.example.com"));
        assert_eq!(tcp.related_port, Some(22222));
    }

    #[test]
    fn ice_candidate_parser_rejects_invalid_fields_without_partial_results() {
        for invalid in [
            "",
            "arbitrary candidate",
            "candidate:",
            "candidate:a 1 udp 0 127.0.0.1 9 typ host",
            "candidate:a 1 udp 2147483648 127.0.0.1 9 typ host",
            "candidate:a 1 udp 1 127.0.0.1 65536 typ host",
            "candidate:a 1 udp 1 127.0.0.1 -1 typ host",
            "candidate:a 3 udp 1 127.0.0.1 9 typ host",
            "candidate:a 1 sctp 1 127.0.0.1 9 typ host",
            "candidate:a 1 udp 1 127.0.0.1 9 typ unknown",
            "candidate:a 1 udp 1 [::1] 9 typ host",
            "candidate:a 1 udp 1 -invalid.local 9 typ host",
            "candidate:a 1 tcp 1 127.0.0.1 9 typ host",
            "candidate:a 1 tcp 1 127.0.0.1 9 typ host tcptype unknown",
            "candidate:a 1 udp 1 127.0.0.1 9 typ host rport 65536",
            "candidate:a 1 udp 1 127.0.0.1 9 typ host generation",
            "candidate:a 1 udp 1 127.0.0.1 9 typ host\r\n",
            "candidate:a 1 udp 1 127.0.0.1 9 typ host x y\n",
        ] {
            assert!(
                parse_ice_candidate(invalid).is_none(),
                "accepted {invalid:?}"
            );
        }
    }
}
