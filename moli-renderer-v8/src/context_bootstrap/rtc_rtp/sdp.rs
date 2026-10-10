pub(super) use moli_webapi::rtc::peer_connection::sdp::session_header;

#[cfg(test)]
mod tests {
    use moli_webapi::rtc::peer_connection::sdp::parse;

    #[test]
    fn sdp_all_compatibility_offers_parse_and_empty_bundle_is_omitted() {
        for bits in 0..8 {
            let text = moli_webapi::rtc::peer_connection::build_signaling_only_offer(
                bits & 1 != 0,
                bits & 2 != 0,
                bits & 4 != 0,
            );
            let parsed = parse(&text).unwrap();
            assert_eq!(parsed.media.len(), (bits as u32).count_ones() as usize);
            assert_eq!(text.contains("a=group:BUNDLE"), bits != 0);
        }
        let text = super::super::rtp_offer::build(&[], false);
        assert!(parse(&text).unwrap().media.is_empty());
        assert!(!text.contains("a=group:BUNDLE"));
    }
}
