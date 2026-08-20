use crate::{RobotsPolicy, RobotsTxt, robots_request_target, robots_txt_url};
use url::Url;

const AGENT: &str = "MoliBot/1.0";

fn allows(robots: &str, request_target: &str) -> bool {
    RobotsTxt::parse(robots).allows(AGENT, request_target)
}

#[test]
fn empty_document_allows_everything() {
    assert!(allows("", "/"));
    assert!(allows("", "/anything/at/all?q=1"));
}

#[test]
fn wildcard_group_applies_when_no_named_group_matches() {
    let robots = "User-agent: *\nDisallow: /private\n";
    assert!(!allows(robots, "/private"));
    assert!(allows(robots, "/public"));
}

#[test]
fn empty_disallow_places_no_restriction() {
    // RFC 9309 §2.2.2: `Disallow:` with no value is the canonical "allow all".
    let robots = "User-agent: *\nDisallow:\n";
    assert!(allows(robots, "/"));
    assert!(allows(robots, "/anything"));
}

#[test]
fn disallow_root_blocks_every_request_target() {
    let robots = "User-agent: *\nDisallow: /\n";
    assert!(!allows(robots, "/"));
    assert!(!allows(robots, "/deep/path?q=1"));
}

#[test]
fn rules_before_any_user_agent_line_bind_nobody() {
    let robots = "Disallow: /\nUser-agent: *\nAllow: /\n";
    assert!(allows(robots, "/orphaned-rule-must-not-apply"));
}

#[test]
fn comments_blank_lines_and_bom_are_tolerated() {
    let robots = "\u{feff}# leading comment\r\n\r\nUser-agent: *   # trailing comment\r\nDisallow: /private   # why\r\n";
    assert!(!allows(robots, "/private"));
    assert!(allows(robots, "/public"));
}

#[test]
fn field_names_are_case_insensitive() {
    let robots = "USER-AGENT: *\nDISALLOW: /private\n";
    assert!(!allows(robots, "/private"));
}

#[test]
fn useragent_spelling_without_hyphen_is_accepted() {
    let robots = "Useragent: *\nDisallow: /private\n";
    assert!(!allows(robots, "/private"));
}

#[test]
fn consecutive_user_agent_lines_share_one_group() {
    let robots = "User-agent: alpha\nUser-agent: molibot\nDisallow: /shared\n";
    assert!(!allows(robots, "/shared"));
}

#[test]
fn a_user_agent_line_after_a_rule_starts_a_new_group() {
    let robots = "User-agent: molibot\nDisallow: /mine\nUser-agent: other\nDisallow: /theirs\n";
    assert!(!allows(robots, "/mine"));
    assert!(allows(robots, "/theirs"));
}

#[test]
fn named_group_wins_over_wildcard_group() {
    let robots = "User-agent: *\nDisallow: /\n\nUser-agent: molibot\nDisallow: /private\n";
    assert!(allows(robots, "/public"));
    assert!(!allows(robots, "/private"));
}

#[test]
fn longest_matching_user_agent_wins() {
    let robots = "User-agent: moli\nDisallow: /\n\nUser-agent: molibot\nDisallow: /private\n";
    // `molibot` is the more specific token, so its narrower rule applies.
    assert!(allows(robots, "/public"));
    assert!(!allows(robots, "/private"));
}

#[test]
fn user_agent_matching_is_case_insensitive() {
    let robots = "User-agent: MOLIBOT\nDisallow: /private\n";
    assert!(!allows(robots, "/private"));
}

#[test]
fn a_product_token_is_found_anywhere_in_the_user_agent() {
    // Compatible-style user agents bury the product token mid-string.
    let robots = "User-agent: ExampleBot\nDisallow: /private\n";
    let agent = "Mozilla/5.0 (compatible; ExampleBot/1.0; +http://example.test/bot)";
    assert!(!RobotsTxt::parse(robots).allows(agent, "/private"));
    assert!(RobotsTxt::parse(robots).allows(agent, "/public"));
}

#[test]
fn repeated_groups_for_one_agent_are_merged() {
    let robots =
        "User-agent: molibot\nDisallow: /first\n\nUser-agent: molibot\nDisallow: /second\n";
    assert!(!allows(robots, "/first"));
    assert!(!allows(robots, "/second"));
    assert!(allows(robots, "/third"));
}

#[test]
fn unknown_fields_are_ignored() {
    let robots = "Sitemap: https://example.test/sitemap.xml\nUser-agent: *\nCrawl-delay: 10\nHost: example.test\nDisallow: /private\n";
    assert!(!allows(robots, "/private"));
    assert!(allows(robots, "/public"));
}

#[test]
fn lines_without_a_separator_are_dropped() {
    let robots = "User-agent: *\nthis line is nonsense\nDisallow: /private\n";
    assert!(!allows(robots, "/private"));
}

#[test]
fn prefix_patterns_follow_the_reference_examples() {
    // The `/fish` cases from Google's robots.txt specification.
    let robots = "User-agent: *\nDisallow: /fish\n";
    for blocked in [
        "/fish",
        "/fish.html",
        "/fish/salmon.html",
        "/fishheads",
        "/fishheads/yummy.html",
        "/fish.php?id=anything",
    ] {
        assert!(!allows(robots, blocked), "{blocked} should be disallowed");
    }
    for permitted in ["/Fish.asp", "/catfish", "/?id=fish", "/desert/fish"] {
        assert!(allows(robots, permitted), "{permitted} should be allowed");
    }
}

#[test]
fn a_trailing_slash_pattern_matches_only_the_directory() {
    let robots = "User-agent: *\nDisallow: /fish/\n";
    for blocked in ["/fish/", "/fish/?id=anything", "/fish/salmon.htm"] {
        assert!(!allows(robots, blocked), "{blocked} should be disallowed");
    }
    for permitted in ["/fish", "/fish.html", "/Fish/Salmon.asp"] {
        assert!(allows(robots, permitted), "{permitted} should be allowed");
    }
}

#[test]
fn wildcards_match_any_run_of_characters() {
    let robots = "User-agent: *\nDisallow: /*.php\n";
    for blocked in [
        "/index.php",
        "/filename.php",
        "/folder/filename.php",
        "/folder/filename.php?parameters",
        "/folder/any.php.file.html",
        "/filename.php/",
    ] {
        assert!(!allows(robots, blocked), "{blocked} should be disallowed");
    }
    for permitted in ["/", "/windows.PHP"] {
        assert!(allows(robots, permitted), "{permitted} should be allowed");
    }
}

#[test]
fn a_trailing_dollar_anchors_the_pattern_to_the_end() {
    let robots = "User-agent: *\nDisallow: /*.php$\n";
    for blocked in ["/filename.php", "/folder/filename.php"] {
        assert!(!allows(robots, blocked), "{blocked} should be disallowed");
    }
    for permitted in [
        "/filename.php?parameters",
        "/filename.php/",
        "/filename.php5",
        "/windows.PHP",
    ] {
        assert!(allows(robots, permitted), "{permitted} should be allowed");
    }
}

#[test]
fn wildcards_combine_with_literal_tails() {
    let robots = "User-agent: *\nDisallow: /fish*.php\n";
    for blocked in ["/fish.php", "/fishheads/catfish.php?parameters"] {
        assert!(!allows(robots, blocked), "{blocked} should be disallowed");
    }
    assert!(allows(robots, "/Fish.PHP"));
}

#[test]
fn a_bare_dollar_pattern_matches_only_the_site_root() {
    let robots = "User-agent: *\nDisallow: /\nAllow: /$\n";
    assert!(allows(robots, "/"));
    assert!(!allows(robots, "/page.htm"));
}

#[test]
fn the_longest_matching_rule_wins() {
    let robots = "User-agent: *\nAllow: /p\nDisallow: /\n";
    assert!(allows(robots, "/page"));
}

#[test]
fn allow_wins_a_tie_against_disallow() {
    let robots = "User-agent: *\nAllow: /folder\nDisallow: /folder\n";
    assert!(allows(robots, "/folder/page"));
}

#[test]
fn rule_order_does_not_change_the_outcome() {
    let forward = "User-agent: *\nDisallow: /folder\nAllow: /folder\n";
    let reverse = "User-agent: *\nAllow: /folder\nDisallow: /folder\n";
    assert_eq!(
        allows(forward, "/folder/page"),
        allows(reverse, "/folder/page")
    );
    assert!(allows(forward, "/folder/page"));
}

#[test]
fn a_narrow_allow_reopens_a_broad_disallow() {
    let robots = "User-agent: *\nDisallow: /admin\nAllow: /admin/public\n";
    assert!(!allows(robots, "/admin/secret"));
    assert!(allows(robots, "/admin/public/page"));
}

#[test]
fn patterns_that_do_not_start_at_the_root_never_match() {
    // Request targets always begin with `/`, so a relative pattern is inert.
    // This mirrors the reference implementation rather than guessing intent.
    let robots = "User-agent: *\nDisallow: private/\n";
    assert!(allows(robots, "/private/"));
}

#[test]
fn the_query_string_is_part_of_the_match() {
    let robots = "User-agent: *\nDisallow: /*?session=\n";
    assert!(!allows(robots, "/page?session=abc"));
    assert!(allows(robots, "/page?other=abc"));
}

#[test]
fn percent_encoding_case_does_not_change_the_match() {
    let robots = "User-agent: *\nDisallow: /caf%c3%a9\n";
    assert!(!allows(robots, "/caf%C3%A9"));
    assert!(!allows(robots, "/caf%c3%a9"));
}

#[test]
fn an_unescaped_pattern_matches_an_escaped_request_target() {
    let robots = "User-agent: *\nDisallow: /café\n";
    assert!(!allows(robots, "/caf%C3%A9"));
}

#[test]
fn a_lone_percent_sign_is_left_alone() {
    let robots = "User-agent: *\nDisallow: /100%discount\n";
    assert!(!allows(robots, "/100%discount"));
}

#[test]
fn http_status_selects_the_policy() {
    let body = "User-agent: *\nDisallow: /\n";

    // 2xx carries rules.
    assert!(!RobotsPolicy::from_http_status(200, body).allows(AGENT, "/page"));
    // 4xx means the origin published no rules.
    assert_eq!(
        RobotsPolicy::from_http_status(404, body),
        RobotsPolicy::AllowAll
    );
    assert_eq!(
        RobotsPolicy::from_http_status(403, body),
        RobotsPolicy::AllowAll
    );
    // 5xx means rules may exist but could not be read.
    assert_eq!(
        RobotsPolicy::from_http_status(503, body),
        RobotsPolicy::DisallowAll
    );
    assert_eq!(
        RobotsPolicy::from_http_status(500, body),
        RobotsPolicy::DisallowAll
    );
}

#[test]
fn an_unreachable_robots_file_disallows_everything() {
    // RFC 9309 §2.3.1.4 makes silence on an unreachable origin a stop, not a go.
    assert!(!RobotsPolicy::unreachable().allows(AGENT, "/"));
}

#[test]
fn allow_all_and_disallow_all_ignore_the_request_target() {
    assert!(RobotsPolicy::AllowAll.allows(AGENT, "/anything"));
    assert!(!RobotsPolicy::DisallowAll.allows(AGENT, "/anything"));
}

#[test]
fn robots_url_is_derived_from_the_origin() {
    let target = Url::parse("https://example.test/deep/page.html?q=1#frag").expect("valid url");
    assert_eq!(
        robots_txt_url(&target).map(String::from).as_deref(),
        Some("https://example.test/robots.txt")
    );
}

#[test]
fn robots_url_keeps_a_non_default_port() {
    let target = Url::parse("http://example.test:8080/page").expect("valid url");
    assert_eq!(
        robots_txt_url(&target).map(String::from).as_deref(),
        Some("http://example.test:8080/robots.txt")
    );
}

#[test]
fn robots_url_drops_credentials() {
    let target = Url::parse("https://user:secret@example.test/page").expect("valid url");
    assert_eq!(
        robots_txt_url(&target).map(String::from).as_deref(),
        Some("https://example.test/robots.txt")
    );
}

#[test]
fn non_http_schemes_carry_no_robots_policy() {
    for raw in ["file:///tmp/page.html", "data:text/html,hi", "about:blank"] {
        let target = Url::parse(raw).expect("valid url");
        assert_eq!(robots_txt_url(&target), None, "{raw} should have no policy");
    }
}

#[test]
fn request_target_carries_the_query_but_not_the_fragment() {
    let target = Url::parse("https://example.test/a/b?c=d#e").expect("valid url");
    assert_eq!(robots_request_target(&target), "/a/b?c=d");

    let bare = Url::parse("https://example.test/a/b").expect("valid url");
    assert_eq!(robots_request_target(&bare), "/a/b");

    let root = Url::parse("https://example.test").expect("valid url");
    assert_eq!(robots_request_target(&root), "/");
}
