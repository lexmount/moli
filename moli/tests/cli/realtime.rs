use super::*;
use moli_core::LayoutPolicy;

#[test]
fn realtime_cli_selects_one_policy_without_enabling_optional_resources() {
    for command in ["fetch", "serve"] {
        for legacy_flag in [false, true] {
            for resources in [false, true] {
                let mut args = vec!["moli", command, "--realtime", "--scrollbars"];
                if legacy_flag {
                    args.push("--layout");
                }
                if resources {
                    args.extend(["--image", "--font"]);
                }
                if command == "fetch" {
                    args.push("about:blank");
                } else {
                    args.extend(["--screencast-interval-ms", "33"]);
                }
                let config = AppConfig::from_cli(&Cli::try_parse_from(args).unwrap()).unwrap();
                assert_eq!(config.browser.layout_policy(), LayoutPolicy::Realtime);
                assert!(!config.browser.scrollbars_hidden());
                assert_eq!(
                    config.browser.optional_resource_fetch_mask(),
                    if resources {
                        OptionalResourceFetchMask::IMAGE | OptionalResourceFetchMask::FONT
                    } else {
                        OptionalResourceFetchMask::NONE
                    }
                );
                if command == "serve" {
                    assert_eq!(config.server.screencast_interval_ms, 33);
                }
            }
        }
        let mut args = vec!["moli", command, "--realtime"];
        if command == "fetch" {
            args.push("about:blank");
        }
        let config = AppConfig::from_cli(&Cli::try_parse_from(args).unwrap()).unwrap();
        assert!(config.browser.scrollbars_hidden());
    }
}

#[test]
fn realtime_environment_and_cli_priority_use_actual_enabled_values() {
    for command in ["fetch", "serve"] {
        for (layout, realtime, flags, expected) in [
            (None, Some("true"), "", "Realtime"),
            (None, Some("1"), "", "Realtime"),
            (None, Some("false"), "", "Mock"),
            (None, Some("0"), "", "Mock"),
            (None, Some("false"), "--realtime", "Realtime"),
            (None, Some("false"), "--layout", "OnDemand"),
            (Some("true"), Some("true"), "", "Realtime"),
            (Some("true"), Some("0"), "", "OnDemand"),
            (Some("false"), Some("true"), "--layout", "Realtime"),
        ] {
            for capability in ["none", "scrollbars", "screencast"] {
                if command == "fetch" && capability == "screencast" {
                    continue;
                }
                let mut child = Command::new(std::env::current_exe().unwrap());
                child
                    .args([
                        "--exact",
                        "realtime::parse_realtime_in_child_process",
                        "--nocapture",
                    ])
                    .env_remove("MOLI_LAYOUT")
                    .env_remove("MOLI_REALTIME")
                    .env_remove("MOLI_SCROLLBARS")
                    .env("MOLI_TEST_REALTIME_COMMAND", command)
                    .env("MOLI_TEST_REALTIME_FLAGS", flags)
                    .env("MOLI_TEST_REALTIME_EXPECTED", expected)
                    .env("MOLI_TEST_REALTIME_CAPABILITY", capability);
                if let Some(value) = layout {
                    child.env("MOLI_LAYOUT", value);
                }
                if let Some(value) = realtime {
                    child.env("MOLI_REALTIME", value);
                }
                let output = child.output().unwrap();
                assert!(
                    output.status.success(),
                    "{command}/{layout:?}/{realtime:?}/{flags}/{capability}: {}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}

#[test]
fn parse_realtime_in_child_process() {
    let Ok(command) = std::env::var("MOLI_TEST_REALTIME_COMMAND") else {
        return;
    };
    let flags = std::env::var("MOLI_TEST_REALTIME_FLAGS").unwrap();
    let expected = std::env::var("MOLI_TEST_REALTIME_EXPECTED").unwrap();
    let capability = std::env::var("MOLI_TEST_REALTIME_CAPABILITY").unwrap();
    let mut args = vec!["moli", command.as_str()];
    args.extend(flags.split_whitespace());
    match capability.as_str() {
        "scrollbars" => args.push("--scrollbars"),
        "screencast" => args.extend(["--screencast-interval-ms", "33"]),
        "none" => {}
        _ => unreachable!(),
    }
    if command == "fetch" {
        args.push("about:blank");
    }
    let result = Cli::try_parse_from(args)
        .map_err(anyhow::Error::from)
        .and_then(|cli| AppConfig::from_cli(&cli));
    if expected == "Mock" && capability != "none" {
        assert!(result.unwrap_err().to_string().contains("--realtime"));
    } else {
        let config = result.unwrap();
        assert_eq!(format!("{:?}", config.browser.layout_policy()), expected);
        assert_eq!(
            config.browser.scrollbars_hidden(),
            expected != "Mock" && capability != "scrollbars"
        );
    }
}
