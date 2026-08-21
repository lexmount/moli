use clap::Parser;
use std::{
    fs,
    num::NonZeroU32,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use moli::cli::{
    Cli, Commands, CommonArgs, DumpFormat, FetchArgs, FetchWaitUntil, LogFormat, LogLevel,
    RequestHeaderArg, ResponseJsonPathArg, ResponseRegexArg, ServeArgs, StripModeChoice,
    StripOptions, WebBotAuthProfileChoice, normalize_args_for_compat,
};
use moli::config::AppConfig;
use moli_browser_profile::BrowserProfilePaths;
use moli_core::OptionalResourceFetchMask;
use moli_fetch::{FetchConfig, WebBotAuthProfile};

const RFC_9421_ED25519_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----\n\
MC4CAQAwBQYDK2VwBCIEIJ+DYvh6SEqVTm50DFtMDoQikTmiCqirVv9mWG9qfSnF\n\
-----END PRIVATE KEY-----\n";
const RFC_9421_ED25519_KEYID: &str = "poqkLGiymh_W0uP6PZFw-dvez3QJT5SolqXBCW38r0U";
static NEXT_TEMP_KEY_FILE: AtomicU64 = AtomicU64::new(0);

#[test]
fn parses_explicit_fetch_command_with_compatibility_flags() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--dump",
        "semantic_tree",
        "--header",
        "X-Test: one",
        "-H",
        "X-Trace: two",
        "--noscript",
        "--with-base",
        "--with-frames",
        "--strip-mode",
        "css,ui",
        "--obey-robots",
        "--http-proxy",
        "http://proxy.internal:8080",
        "--proxy-bearer-token",
        "secret",
        "--http-max-concurrent",
        "256",
        "--http-max-host-open",
        "256",
        "--http-max-host-connections",
        "6",
        "--http-max-total-connections",
        "64",
        "--http2-max-concurrent-streams",
        "100",
        "--http-connect-timeout",
        "1200",
        "--http-timeout",
        "5000",
        "--http-max-response-size",
        "4096",
        "--log-level",
        "info",
        "--log-format",
        "pretty",
        "--log-filter-scopes",
        "http,event",
        "--user-agent-suffix",
        "internal-tester",
        "https://example.com",
    ]))
    .unwrap();

    assert_eq!(
        cli.command,
        Commands::Fetch(Box::new(FetchArgs {
            dump: Some(DumpFormat::SemanticTree),
            headers: vec![
                RequestHeaderArg {
                    name: "X-Test".to_owned(),
                    value: "one".to_owned(),
                },
                RequestHeaderArg {
                    name: "X-Trace".to_owned(),
                    value: "two".to_owned(),
                },
            ],
            noscript: true,
            with_base: true,
            with_frames: true,
            trace_network: false,
            trace_matched_response_body: false,
            strip_mode: vec![StripModeChoice::Css, StripModeChoice::Ui],
            wait_until: FetchWaitUntil::Done,
            redirect_wait_ms: 1_000,
            wait_selector: None,
            wait_script: None,
            wait_script_file: None,
            delay_ms: 0,
            wait_response_url: None,
            wait_response_url_regex: None,
            wait_response_body: None,
            wait_response_body_regex: None,
            wait_response_json: None,
            wait_response_json_regex: None,
            timeout: 25_000,
            common: CommonArgs {
                insecure_disable_tls_host_verification: false,
                obey_robots: true,
                http_proxy: Some("http://proxy.internal:8080".to_owned()),
                http_no_proxy: None,
                http_host_resolve: Vec::new(),
                proxy_bearer_token: Some("secret".to_owned()),
                http_max_concurrent: NonZeroU32::new(256),
                http_max_host_open: NonZeroU32::new(256),
                http_max_host_connections: Some(6),
                http_max_total_connections: Some(64),
                http2_max_concurrent_streams: Some(100),
                http_connect_timeout: Some(1200),
                http_timeout: Some(5000),
                http_max_response_size: Some(4096),
                http_cache_dir: None,
                profile_dir: None,
                image: false,
                font: false,
                audio: false,
                video: false,
                media: false,
                text_track: false,
                resource: false,
                disable_subframes: false,
                layout: false,
                cookie_file: Vec::new(),
                document_start_script: Vec::new(),
                document_start_script_file: Vec::new(),
                block_private_networks: false,
                block_cidrs: None,
                log_level: Some(LogLevel::Info),
                log_format: Some(LogFormat::Pretty),
                log_filter_scopes: Some("http,event".to_owned()),
                user_agent: None,
                user_agent_suffix: Some("internal-tester".to_owned()),
                web_bot_auth_key_file: None,
                web_bot_auth_keyid: None,
                web_bot_auth_domain: None,
                web_bot_auth_profile: WebBotAuthProfileChoice::Cloudflare,
            },
            url: "https://example.com".to_owned(),
        }))
    );
}

#[test]
fn rejects_zero_runtime_transfer_limits() {
    for flag in ["--http-max-concurrent", "--http-max-host-open"] {
        let err = Cli::try_parse_from(normalize_args_for_compat([
            "moli",
            "fetch",
            flag,
            "0",
            "https://example.com",
        ]))
        .unwrap_err();

        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }
}

#[test]
fn parses_json_dump_mode() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--dump",
        "json",
        "https://example.com",
    ]))
    .unwrap();

    let Commands::Fetch(args) = cli.command else {
        panic!("expected fetch command");
    };

    assert_eq!(args.dump, Some(DumpFormat::Json));
}

#[test]
fn rejects_removed_wpt_dump_mode() {
    let result = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--dump",
        "wpt",
        "https://example.com",
    ]));

    assert!(result.is_err());
}

#[test]
fn parses_binary_dump_modes_with_explicit_fetch_command() {
    for (value, expected) in [
        ("screenshot", DumpFormat::Screenshot),
        ("screenshot_full", DumpFormat::ScreenshotFull),
        ("pdf", DumpFormat::Pdf),
    ] {
        let cli = Cli::try_parse_from(normalize_args_for_compat([
            "moli",
            "fetch",
            "--dump",
            value,
            "https://example.com",
        ]))
        .unwrap();

        let Commands::Fetch(args) = cli.command else {
            panic!("expected fetch command for --dump {value}");
        };
        assert_eq!(args.dump, Some(expected));
    }
}

#[test]
fn parses_web_bot_auth_flags() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--web-bot-auth-key-file",
        "bot-key.pem",
        "--web-bot-auth-keyid",
        RFC_9421_ED25519_KEYID,
        "--web-bot-auth-domain",
        "bot.example",
        "--web-bot-auth-profile",
        "ietf-01",
        "https://example.com",
    ]))
    .unwrap();

    let Commands::Fetch(args) = cli.command else {
        panic!("expected fetch command");
    };
    assert_eq!(
        args.common.web_bot_auth_key_file.as_deref(),
        Some("bot-key.pem")
    );
    assert_eq!(
        args.common.web_bot_auth_keyid.as_deref(),
        Some(RFC_9421_ED25519_KEYID)
    );
    assert_eq!(
        args.common.web_bot_auth_domain.as_deref(),
        Some("bot.example")
    );
    assert_eq!(
        args.common.web_bot_auth_profile,
        WebBotAuthProfileChoice::IetfDraft01
    );
}

#[test]
fn web_bot_auth_profile_default_does_not_enable_signing() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "https://example.com",
    ]))
    .unwrap();

    let Commands::Fetch(args) = cli.command else {
        panic!("expected fetch command");
    };
    assert_eq!(
        args.common.web_bot_auth_profile,
        WebBotAuthProfileChoice::Cloudflare
    );
    assert!(args.common.web_bot_auth_key_file.is_none());
}

#[test]
fn web_bot_auth_flags_require_key_and_domain_together() {
    for args in [
        vec!["--web-bot-auth-key-file", "bot-key.pem"],
        vec!["--web-bot-auth-domain", "bot.example"],
        vec!["--web-bot-auth-keyid", RFC_9421_ED25519_KEYID],
        vec!["--web-bot-auth-profile", "ietf-01"],
    ] {
        let mut command = vec!["moli", "fetch"];
        command.extend(args);
        command.push("https://example.com");
        let error = Cli::try_parse_from(normalize_args_for_compat(command)).unwrap_err();

        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }
}

#[test]
fn app_config_accepts_cookie_file_for_cdp_serve() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "serve",
        "--cookie-file",
        "/tmp/browser-cookies.txt",
    ]))
    .unwrap();

    let config = AppConfig::from_cli(&cli).unwrap();
    assert_eq!(
        config.fetch.cookie_files,
        vec!["/tmp/browser-cookies.txt".to_owned()]
    );
}

#[test]
fn parses_trace_network_fetch_flag() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--trace-network",
        "--dump",
        "json",
        "https://example.com",
    ]))
    .unwrap();

    let Commands::Fetch(args) = cli.command else {
        panic!("expected fetch command");
    };

    assert!(args.trace_network);
    assert!(!args.trace_matched_response_body);
    assert_eq!(args.dump, Some(DumpFormat::Json));
}

#[test]
fn parses_trace_matched_response_body_fetch_flag() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--trace-network",
        "--trace-matched-response-body",
        "--dump",
        "json",
        "https://example.com",
    ]))
    .unwrap();

    let Commands::Fetch(args) = cli.command else {
        panic!("expected fetch command");
    };

    assert!(args.trace_network);
    assert!(args.trace_matched_response_body);
    assert_eq!(args.dump, Some(DumpFormat::Json));
}

#[test]
fn trace_matched_response_body_requires_trace_network() {
    let err = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--trace-matched-response-body",
        "--dump",
        "json",
        "https://example.com",
    ]))
    .unwrap_err();

    assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
}

#[test]
fn infers_fetch_mode_from_bare_url() {
    let cli =
        Cli::try_parse_from(normalize_args_for_compat(["moli", "https://example.com"])).unwrap();

    assert_eq!(
        cli.command,
        Commands::Fetch(Box::new(FetchArgs {
            dump: None,
            headers: vec![],
            noscript: false,
            with_base: false,
            with_frames: false,
            trace_network: false,
            trace_matched_response_body: false,
            strip_mode: vec![],
            wait_until: FetchWaitUntil::Done,
            redirect_wait_ms: 1_000,
            wait_selector: None,
            wait_script: None,
            wait_script_file: None,
            delay_ms: 0,
            wait_response_url: None,
            wait_response_url_regex: None,
            wait_response_body: None,
            wait_response_body_regex: None,
            wait_response_json: None,
            wait_response_json_regex: None,
            timeout: 25_000,
            common: CommonArgs::default(),
            url: "https://example.com".to_owned(),
        }))
    );
}

#[test]
fn parses_bare_dump_with_explicit_fetch_command_and_defaults_to_html() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--dump",
        "https://example.com",
    ]))
    .unwrap();

    assert_eq!(
        cli.command,
        Commands::Fetch(Box::new(FetchArgs {
            dump: Some(DumpFormat::Html),
            headers: vec![],
            noscript: false,
            with_base: false,
            with_frames: false,
            trace_network: false,
            trace_matched_response_body: false,
            strip_mode: vec![],
            wait_until: FetchWaitUntil::Done,
            redirect_wait_ms: 1_000,
            wait_selector: None,
            wait_script: None,
            wait_script_file: None,
            delay_ms: 0,
            wait_response_url: None,
            wait_response_url_regex: None,
            wait_response_body: None,
            wait_response_body_regex: None,
            wait_response_json: None,
            wait_response_json_regex: None,
            timeout: 25_000,
            common: CommonArgs::default(),
            url: "https://example.com".to_owned(),
        }))
    );
}

#[test]
fn parses_header_flag_with_explicit_fetch_command() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "-H",
        "X-Test: one",
        "https://example.com",
    ]))
    .unwrap();

    assert_eq!(
        cli.command,
        Commands::Fetch(Box::new(FetchArgs {
            dump: None,
            headers: vec![RequestHeaderArg {
                name: "X-Test".to_owned(),
                value: "one".to_owned(),
            }],
            noscript: false,
            with_base: false,
            with_frames: false,
            trace_network: false,
            trace_matched_response_body: false,
            strip_mode: vec![],
            wait_until: FetchWaitUntil::Done,
            redirect_wait_ms: 1_000,
            wait_selector: None,
            wait_script: None,
            wait_script_file: None,
            delay_ms: 0,
            wait_response_url: None,
            wait_response_url_regex: None,
            wait_response_body: None,
            wait_response_body_regex: None,
            wait_response_json: None,
            wait_response_json_regex: None,
            timeout: 25_000,
            common: CommonArgs::default(),
            url: "https://example.com".to_owned(),
        }))
    );
}

#[test]
fn every_optional_resource_flag_parses_with_explicit_fetch_command() {
    let cases = [
        ("--image", 0),
        ("--font", 1),
        ("--audio", 2),
        ("--video", 3),
        ("--media", 4),
        ("--text-track", 5),
        ("--resource", 6),
    ];

    for (flag, enabled_index) in cases {
        let cli = Cli::try_parse_from(normalize_args_for_compat([
            "moli",
            "fetch",
            flag,
            "https://example.com",
        ]))
        .unwrap_or_else(|error| panic!("{flag} should parse for fetch: {error}"));
        let Commands::Fetch(args) = cli.command else {
            panic!("{flag} should preserve the explicit fetch command");
        };
        let values = [
            args.common.image,
            args.common.font,
            args.common.audio,
            args.common.video,
            args.common.media,
            args.common.text_track,
            args.common.resource,
        ];

        assert_eq!(args.url, "https://example.com");
        for (index, value) in values.into_iter().enumerate() {
            assert_eq!(
                value,
                index == enabled_index,
                "{flag} unexpectedly changed flag index {index}"
            );
        }
    }
}

#[test]
fn parses_disable_subframes_with_explicit_fetch_command() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--disable-subframes",
        "https://example.com",
    ]))
    .unwrap();

    let Commands::Fetch(args) = cli.command else {
        panic!("expected fetch command");
    };

    assert!(args.common.disable_subframes);
    assert_eq!(args.url, "https://example.com");
}

#[test]
fn infers_serve_mode_when_called_without_args() {
    let cli = Cli::try_parse_from(normalize_args_for_compat(["moli"])).unwrap();

    assert_eq!(
        cli.command,
        Commands::Serve(Box::new(ServeArgs {
            host: "127.0.0.1".to_owned(),
            port: 9222,
            timeout: 10,
            cdp_max_connections: 16,
            cdp_max_pending_connections: 128,
            common: CommonArgs::default(),
        }))
    );
}

#[test]
fn parses_serve_flags_with_explicit_command() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "serve",
        "--host",
        "0.0.0.0",
        "--port",
        "9333",
        "--timeout",
        "42",
    ]))
    .unwrap();

    assert_eq!(
        cli.command,
        Commands::Serve(Box::new(ServeArgs {
            host: "0.0.0.0".to_owned(),
            port: 9333,
            timeout: 42,
            cdp_max_connections: 16,
            cdp_max_pending_connections: 128,
            common: CommonArgs::default(),
        }))
    );
}

#[test]
fn rejects_removed_version_and_help_subcommands() {
    for command in ["version", "help"] {
        let error = Cli::try_parse_from(["moli", command]).unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidSubcommand);
    }
}

#[test]
fn standard_version_and_help_flags_remain_available() {
    for (flag, expected_kind) in [
        ("--version", clap::error::ErrorKind::DisplayVersion),
        ("--help", clap::error::ErrorKind::DisplayHelp),
    ] {
        let error = Cli::try_parse_from(["moli", flag]).unwrap_err();
        assert_eq!(error.kind(), expected_kind);
    }
}

#[test]
fn app_config_uses_moli_user_agent_defaults() {
    let config = AppConfig::default();
    assert_eq!(
        config.browser.fetch().user_agent(),
        FetchConfig::DEFAULT_USER_AGENT
    );

    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--user-agent-suffix",
        "internal-tester",
        "https://example.com",
    ]))
    .unwrap();
    let config = AppConfig::from_cli(&cli).unwrap();
    assert_eq!(
        config.browser.fetch().user_agent(),
        format!("{} internal-tester", FetchConfig::DEFAULT_USER_AGENT)
    );

    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--user-agent",
        "ExampleBrowser/1.0",
        "--user-agent-suffix",
        "ignored",
        "https://example.com",
    ]))
    .unwrap();
    let config = AppConfig::from_cli(&cli).unwrap();
    assert_eq!(config.browser.fetch().user_agent(), "ExampleBrowser/1.0");
}

#[test]
fn app_config_loads_and_validates_web_bot_auth_key() {
    let key_file = write_temp_web_bot_auth_key();
    let key_path_arg = key_file.path.to_string_lossy().into_owned();
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--web-bot-auth-key-file",
        key_path_arg.as_str(),
        "--web-bot-auth-keyid",
        RFC_9421_ED25519_KEYID,
        "--web-bot-auth-domain",
        "bot.example:8443",
        "--web-bot-auth-profile",
        "ietf-01",
        "https://example.com",
    ]))
    .unwrap();

    let config = AppConfig::from_cli(&cli).unwrap();
    let signer = config.browser.fetch().web_bot_auth().unwrap();
    assert_eq!(signer.keyid(), RFC_9421_ED25519_KEYID);
    assert_eq!(signer.signature_agent_origin(), "https://bot.example:8443");
    assert_eq!(signer.profile(), WebBotAuthProfile::IetfDraft01);
}

#[test]
fn app_config_rejects_mismatched_web_bot_auth_keyid() {
    let key_file = write_temp_web_bot_auth_key();
    let key_path_arg = key_file.path.to_string_lossy().into_owned();
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--web-bot-auth-key-file",
        key_path_arg.as_str(),
        "--web-bot-auth-keyid",
        "wrong-thumbprint",
        "--web-bot-auth-domain",
        "bot.example",
        "https://example.com",
    ]))
    .unwrap();

    let error = AppConfig::from_cli(&cli).unwrap_err().to_string();
    assert!(error.contains("does not match the private key"));
    assert!(error.contains(RFC_9421_ED25519_KEYID));
}

#[test]
fn app_config_preserves_repeatable_request_headers() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--header",
        "X-Test: one",
        "-H",
        "X-Trace: two",
        "https://example.com",
    ]))
    .unwrap();
    let config = AppConfig::from_cli(&cli).unwrap();

    assert_eq!(
        config.fetch.request_headers,
        &[
            ("X-Test".to_owned(), "one".to_owned()),
            ("X-Trace".to_owned(), "two".to_owned()),
        ]
    );
    assert!(config.browser.fetch().default_request_headers().is_empty());
}

struct TempWebBotAuthKeyFile {
    path: PathBuf,
}

impl Drop for TempWebBotAuthKeyFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn write_temp_web_bot_auth_key() -> TempWebBotAuthKeyFile {
    let sequence = NEXT_TEMP_KEY_FILE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "moli-web-bot-auth-{}-{sequence}.pem",
        std::process::id()
    ));
    fs::write(&path, RFC_9421_ED25519_PRIVATE_KEY).unwrap();
    TempWebBotAuthKeyFile { path }
}

#[test]
fn parses_request_headers_with_embedded_colons_and_empty_values() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--header",
        "Authorization: Bearer a:b:c",
        "--header",
        "X-Empty:",
        "https://example.com",
    ]))
    .unwrap();

    let config = AppConfig::from_cli(&cli).unwrap();
    assert_eq!(
        config.fetch.request_headers,
        &[
            ("Authorization".to_owned(), "Bearer a:b:c".to_owned()),
            ("X-Empty".to_owned(), "".to_owned()),
        ]
    );
    assert!(config.browser.fetch().default_request_headers().is_empty());
}

#[test]
fn rejects_request_header_without_separator() {
    let error = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--header",
        "X-Test",
        "https://example.com",
    ]))
    .unwrap_err();

    assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
}

#[test]
fn fetch_strip_options_combine_cli_selections() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--noscript",
        "--strip-mode",
        "ui,css",
        "https://example.com",
    ]))
    .unwrap();

    let Commands::Fetch(args) = cli.command else {
        panic!("expected fetch command");
    };

    assert_eq!(
        args.strip_options(),
        StripOptions {
            js: true,
            ui: true,
            css: true,
        }
    );
}

#[test]
fn does_not_infer_subcommand_from_flag() {
    let error =
        Cli::try_parse_from(normalize_args_for_compat(["moli", "--obey-robots"])).unwrap_err();

    assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
}

#[test]
fn parses_fetch_wait_until_and_timeout() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--wait-until",
        "networkidle",
        "--timeout",
        "1500",
        "https://example.com",
    ]))
    .unwrap();

    let Commands::Fetch(args) = cli.command else {
        panic!("expected fetch command");
    };

    assert_eq!(args.wait_until, FetchWaitUntil::NetworkIdle);
    assert_eq!(args.timeout, 1500);
}

#[test]
fn parses_dcl_fetch_wait_until_alias() {
    for value in ["domcontentloaded", "dcl"] {
        let cli = Cli::try_parse_from(normalize_args_for_compat([
            "moli",
            "fetch",
            "--wait-until",
            value,
            "https://example.com",
        ]))
        .unwrap();

        let Commands::Fetch(args) = cli.command else {
            panic!("expected fetch command");
        };

        assert_eq!(args.wait_until, FetchWaitUntil::DomContentLoaded);
    }
}

#[test]
fn parses_fetch_literal_response_wait_flags() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--wait-response-url",
        "mtop.taobao.idle.pc.detail",
        "--wait-response-body",
        "SUCCESS",
        "--wait-response-json",
        "data.url=/item/42",
        "https://example.com",
    ]))
    .unwrap();

    let Commands::Fetch(args) = cli.command else {
        panic!("expected fetch command");
    };

    assert_eq!(
        args.wait_response_url.as_deref(),
        Some("mtop.taobao.idle.pc.detail")
    );
    assert_eq!(args.wait_response_body.as_deref(), Some("SUCCESS"));
    assert_eq!(
        args.wait_response_json,
        Some(ResponseJsonPathArg {
            path: vec!["data".to_owned(), "url".to_owned()],
            expected: "/item/42".to_owned(),
        })
    );
    assert!(args.wait_response_url_regex.is_none());
    assert!(args.wait_response_body_regex.is_none());
    assert!(args.wait_response_json_regex.is_none());
}

#[test]
fn parses_fetch_regex_response_wait_flags() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--wait-response-url-regex",
        r"mtop\.taobao\..*\.detail",
        "--wait-response-body-regex",
        "SUCC[A-Z]+",
        "--wait-response-json-regex",
        r"data.url=^/item/\d+$",
        "https://example.com",
    ]))
    .unwrap();

    let Commands::Fetch(args) = cli.command else {
        panic!("expected fetch command");
    };

    assert_eq!(
        args.wait_response_url_regex
            .as_ref()
            .map(ResponseRegexArg::pattern),
        Some(r"mtop\.taobao\..*\.detail")
    );
    assert_eq!(
        args.wait_response_body_regex
            .as_ref()
            .map(ResponseRegexArg::pattern),
        Some("SUCC[A-Z]+")
    );
    let json = args
        .wait_response_json_regex
        .as_ref()
        .expect("JSON regex wait argument");
    assert_eq!(json.path, ["data".to_owned(), "url".to_owned()]);
    assert_eq!(json.pattern(), r"^/item/\d+$");
    assert!(args.wait_response_url.is_none());
    assert!(args.wait_response_body.is_none());
    assert!(args.wait_response_json.is_none());
}

#[test]
fn rejects_invalid_response_wait_regexes() {
    let cases = [
        (
            "--wait-response-url-regex",
            "[",
            "invalid --wait-response-url-regex regex",
        ),
        (
            "--wait-response-body-regex",
            "[",
            "invalid --wait-response-body-regex regex",
        ),
        (
            "--wait-response-json-regex",
            "data.url=[",
            "invalid --wait-response-json-regex regex",
        ),
    ];

    for (flag, value, expected) in cases {
        let error = Cli::try_parse_from(normalize_args_for_compat([
            "moli",
            "fetch",
            flag,
            value,
            "https://example.com",
        ]))
        .unwrap_err();

        assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn literal_and_regex_response_wait_flags_conflict_per_dimension() {
    let cases = [
        (
            "--wait-response-url",
            "/api/items",
            "--wait-response-url-regex",
            "/api/.*",
        ),
        (
            "--wait-response-body",
            "ready",
            "--wait-response-body-regex",
            "ready|done",
        ),
        (
            "--wait-response-json",
            "data.state=ready",
            "--wait-response-json-regex",
            "data.state=ready|done",
        ),
    ];

    for (literal_flag, literal, regex_flag, regex) in cases {
        let error = Cli::try_parse_from(normalize_args_for_compat([
            "moli",
            "fetch",
            literal_flag,
            literal,
            regex_flag,
            regex,
            "https://example.com",
        ]))
        .unwrap_err();

        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }
}

#[test]
fn parses_fetch_domstable_wait_until() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--wait-until",
        "domstable",
        "https://example.com",
    ]))
    .unwrap();

    let Commands::Fetch(args) = cli.command else {
        panic!("expected fetch command");
    };

    assert_eq!(args.wait_until, FetchWaitUntil::DomStable);
}

#[test]
fn parses_wait_flags_with_explicit_fetch_command() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--wait-selector",
        "#ready",
        "https://example.com",
    ]))
    .unwrap();

    let Commands::Fetch(args) = cli.command else {
        panic!("expected fetch command");
    };

    assert_eq!(args.wait_selector.as_deref(), Some("#ready"));
    assert_eq!(args.wait_until, FetchWaitUntil::Done);
}

#[test]
fn parses_fetch_network_policy_flags() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--http-no-proxy",
        "localhost,127.0.0.1",
        "--http-host-resolve",
        "web-platform.test:8443:127.0.0.1",
        "--http-host-resolve",
        "not-web-platform.test:8000:127.0.0.1",
        "--http-cache-dir",
        "/tmp/moli-cache",
        "--cookie-file",
        "/tmp/browser-cookies.txt",
        "--cookie-file",
        "/tmp/extra-cookies.txt",
        "--block-private-networks",
        "--block-cidrs",
        "198.18.0.0/15,203.0.113.0/24",
        "https://example.com",
    ]))
    .unwrap();

    let Commands::Fetch(args) = cli.command else {
        panic!("expected fetch command");
    };

    assert_eq!(
        args.common.http_no_proxy.as_deref(),
        Some("localhost,127.0.0.1")
    );
    assert_eq!(
        args.common.http_host_resolve,
        [
            "web-platform.test:8443:127.0.0.1".to_owned(),
            "not-web-platform.test:8000:127.0.0.1".to_owned()
        ]
    );
    assert_eq!(
        args.common.http_cache_dir.as_deref(),
        Some("/tmp/moli-cache")
    );
    assert_eq!(
        args.common.cookie_file,
        [
            "/tmp/browser-cookies.txt".to_owned(),
            "/tmp/extra-cookies.txt".to_owned()
        ]
    );
    assert!(args.common.block_private_networks);
    assert_eq!(
        args.common.block_cidrs.as_deref(),
        Some("198.18.0.0/15,203.0.113.0/24")
    );
}

#[test]
fn app_config_from_cli_applies_http_host_resolve_entries() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "serve",
        "--http-host-resolve",
        "web-platform.test:8443:127.0.0.1",
        "--http-host-resolve",
        "not-web-platform.test:8000:127.0.0.1",
    ]))
    .unwrap();

    let config = AppConfig::from_cli(&cli).unwrap();
    assert_eq!(
        config.browser.fetch().http_host_resolve(),
        [
            "web-platform.test:8443:127.0.0.1".to_owned(),
            "not-web-platform.test:8000:127.0.0.1".to_owned()
        ]
    );
}

#[test]
fn app_config_rejects_invalid_http_host_resolve_entry() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "serve",
        "--http-host-resolve",
        "web-platform.test:not-a-port:127.0.0.1",
    ]))
    .unwrap();

    let error = AppConfig::from_cli(&cli).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("invalid --http-host-resolve port"),
        "{error:#}"
    );
}

#[test]
fn removed_cookie_cache_flag_is_rejected() {
    let error = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--cookie-cache-file",
        "/tmp/moli-cookies.json",
        "https://example.com",
    ]))
    .unwrap_err();

    assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
}

#[test]
fn app_config_profile_dir_sets_default_http_cache_root() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--profile-dir",
        "/tmp/moli-profile",
        "https://example.com",
    ]))
    .unwrap();

    let config = AppConfig::from_cli(&cli).unwrap();
    let profile = BrowserProfilePaths::new("/tmp/moli-profile");
    assert_eq!(
        config.browser.fetch().http_cache_dir(),
        Some(profile.http_cache_root.to_string_lossy().as_ref())
    );
}

#[test]
fn app_config_explicit_http_cache_dir_overrides_profile_default() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--profile-dir",
        "/tmp/moli-profile",
        "--http-cache-dir",
        "/tmp/moli-cache",
        "https://example.com",
    ]))
    .unwrap();

    let config = AppConfig::from_cli(&cli).unwrap();
    assert_eq!(
        config.browser.fetch().http_cache_dir(),
        Some("/tmp/moli-cache")
    );
}

#[test]
fn app_config_defaults_fetch_http_timeout_to_30_seconds() {
    let config = AppConfig::default();
    assert_eq!(config.browser.fetch().request_timeout_ms(), 30_000);
}

#[test]
fn app_config_from_cli_applies_fetch_http_timeout_override() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--http-timeout",
        "1200",
        "https://example.com",
    ]))
    .unwrap();

    let config = AppConfig::from_cli(&cli).unwrap();
    assert_eq!(config.browser.fetch().request_timeout_ms(), 1200);
}

#[test]
fn app_config_from_fetch_cli_enables_image_fetch() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--image",
        "https://example.com",
    ]))
    .unwrap();

    let config = AppConfig::from_cli(&cli).unwrap();
    assert!(config.browser.image_fetch_enabled());
}

#[test]
fn app_config_from_serve_cli_enables_image_fetch() {
    let cli = Cli::try_parse_from(normalize_args_for_compat(["moli", "serve", "--image"])).unwrap();

    let config = AppConfig::from_cli(&cli).unwrap();
    assert!(config.browser.image_fetch_enabled());
}

#[test]
fn app_config_defaults_all_optional_resource_fetches_off_for_fetch_and_serve() {
    for args in [
        vec!["moli", "fetch", "https://example.com"],
        vec!["moli", "serve"],
    ] {
        let cli = Cli::try_parse_from(normalize_args_for_compat(args.clone())).unwrap();
        let config = AppConfig::from_cli(&cli).unwrap();
        assert_eq!(
            config.browser.optional_resource_fetch_mask(),
            OptionalResourceFetchMask::NONE,
            "default command {args:?} enabled an optional resource"
        );
    }
}

#[test]
fn app_config_defaults_layout_to_mock_for_fetch_and_serve() {
    for args in [
        vec!["moli", "fetch", "https://example.com"],
        vec!["moli", "serve"],
    ] {
        let cli = Cli::try_parse_from(normalize_args_for_compat(args)).unwrap();
        let config = AppConfig::from_cli(&cli).unwrap();
        assert_eq!(
            config.browser.layout_policy(),
            moli_core::LayoutPolicy::Mock
        );
    }
}

#[test]
fn layout_selects_on_demand_policy_for_fetch_and_serve() {
    for args in [
        vec!["moli", "fetch", "--layout", "https://example.com"],
        vec!["moli", "serve", "--layout"],
    ] {
        let cli = Cli::try_parse_from(normalize_args_for_compat(args)).unwrap();
        let config = AppConfig::from_cli(&cli).unwrap();
        assert_eq!(
            config.browser.layout_policy(),
            moli_core::LayoutPolicy::OnDemand
        );
    }
}

#[test]
fn env_flags_are_fallbacks_and_cli_flags_take_priority() {
    for (case, env_value, expected) in [
        ("env-enables", "true", true),
        ("env-disables", "false", false),
        ("env-one-enables", "1", true),
        ("env-zero-disables", "0", false),
        ("cli-overrides-env", "0", true),
    ] {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "parse_env_flags_in_child_process", "--nocapture"])
            .env("MOLI_TEST_ENV_FLAG_CASE", case)
            .env("MOLI_TEST_ENV_FLAG_EXPECTED", expected.to_string())
            .env("MOLI_LAYOUT", env_value)
            .env("MOLI_RESOURCE", env_value)
            .env("MOLI_BLOCK_PRIVATE_NETWORKS", env_value)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "environment flag case {case:?} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

#[test]
fn parse_env_flags_in_child_process() {
    let Ok(case) = std::env::var("MOLI_TEST_ENV_FLAG_CASE") else {
        return;
    };
    let expected = std::env::var("MOLI_TEST_ENV_FLAG_EXPECTED")
        .unwrap()
        .parse::<bool>()
        .unwrap();

    let args = if case == "cli-overrides-env" {
        vec![
            "moli",
            "serve",
            "--layout",
            "--resource",
            "--block-private-networks",
        ]
    } else {
        vec!["moli", "serve"]
    };
    let cli = Cli::try_parse_from(normalize_args_for_compat(args)).unwrap();
    let Commands::Serve(args) = cli.command else {
        panic!("expected serve command");
    };
    assert_eq!(args.common.layout, expected);
    assert_eq!(args.common.resource, expected);
    assert_eq!(args.common.block_private_networks, expected);
}

#[test]
fn removed_long_form_flags_are_rejected() {
    for flag in [
        "--no-layout",
        "--enable-all-resource-fetch",
        "--enable-image-fetch",
        "--enable-font-fetch",
        "--enable-audio-fetch",
        "--enable-video-fetch",
        "--enable-media-fetch",
        "--enable-text-track-fetch",
    ] {
        let error = Cli::try_parse_from(["moli", "serve", flag]).unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }
}

#[test]
fn each_optional_resource_flag_sets_exactly_one_mask_bit_for_fetch_and_serve() {
    let cases = [
        ("--image", OptionalResourceFetchMask::IMAGE),
        ("--font", OptionalResourceFetchMask::FONT),
        ("--audio", OptionalResourceFetchMask::AUDIO),
        ("--video", OptionalResourceFetchMask::VIDEO),
        ("--media", OptionalResourceFetchMask::MEDIA),
        ("--text-track", OptionalResourceFetchMask::TEXT_TRACK),
    ];

    for command in ["fetch", "serve"] {
        for (flag, expected) in cases {
            let mut args = vec!["moli", command, flag];
            if command == "fetch" {
                args.push("https://example.com");
            }
            let cli = Cli::try_parse_from(normalize_args_for_compat(args)).unwrap();
            let config = AppConfig::from_cli(&cli).unwrap();
            assert_eq!(
                config.browser.optional_resource_fetch_mask(),
                expected,
                "{command} {flag} must not alias another optional resource bit"
            );
        }
    }
}

#[test]
fn representative_optional_resource_flag_subsets_compose_for_fetch_and_serve() {
    let cases: [(&[&str], OptionalResourceFetchMask); 4] = [
        (
            &["--image", "--font"],
            OptionalResourceFetchMask::IMAGE | OptionalResourceFetchMask::FONT,
        ),
        (
            &["--audio", "--video"],
            OptionalResourceFetchMask::AUDIO | OptionalResourceFetchMask::VIDEO,
        ),
        (
            &["--font", "--media", "--text-track"],
            OptionalResourceFetchMask::FONT
                | OptionalResourceFetchMask::MEDIA
                | OptionalResourceFetchMask::TEXT_TRACK,
        ),
        (
            &["--image", "--audio", "--video", "--text-track"],
            OptionalResourceFetchMask::IMAGE
                | OptionalResourceFetchMask::AUDIO
                | OptionalResourceFetchMask::VIDEO
                | OptionalResourceFetchMask::TEXT_TRACK,
        ),
    ];

    for command in ["fetch", "serve"] {
        for (flags, expected) in cases {
            let mut args = vec!["moli", command];
            args.extend_from_slice(flags);
            if command == "fetch" {
                args.push("https://example.com");
            }
            let cli = Cli::try_parse_from(normalize_args_for_compat(args)).unwrap();
            let config = AppConfig::from_cli(&cli).unwrap();
            assert_eq!(
                config.browser.optional_resource_fetch_mask(),
                expected,
                "{command} subset {flags:?} produced the wrong optional-resource mask"
            );
        }
    }
}

#[test]
fn all_resource_flag_enables_the_full_mask_for_fetch_and_serve() {
    for command in ["fetch", "serve"] {
        let mut args = vec!["moli", command, "--resource"];
        if command == "fetch" {
            args.push("https://example.com");
        }
        let cli = Cli::try_parse_from(normalize_args_for_compat(args)).unwrap();
        let config = AppConfig::from_cli(&cli).unwrap();
        assert_eq!(
            config.browser.optional_resource_fetch_mask(),
            OptionalResourceFetchMask::ALL
        );
    }
}

#[test]
fn app_config_from_fetch_cli_disables_subframes() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--disable-subframes",
        "https://example.com",
    ]))
    .unwrap();

    let config = AppConfig::from_cli(&cli).unwrap();
    assert!(!config.browser.subframe_loading_enabled());
}

#[test]
fn app_config_from_serve_cli_disables_subframes() {
    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "serve",
        "--disable-subframes",
    ]))
    .unwrap();

    let config = AppConfig::from_cli(&cli).unwrap();
    assert!(!config.browser.subframe_loading_enabled());
}

#[test]
fn app_config_document_start_script_helpers_forward_to_browser_config() {
    let mut config = AppConfig::default();
    config.add_document_start_script("globalThis.__a = 1;");
    assert_eq!(
        config.document_start_scripts(),
        &["globalThis.__a = 1;".to_owned()]
    );

    let config = config.with_document_start_script("globalThis.__b = 2;");
    assert_eq!(
        config.document_start_scripts(),
        &[
            "globalThis.__a = 1;".to_owned(),
            "globalThis.__b = 2;".to_owned(),
        ]
    );
}

#[test]
fn app_config_from_cli_loads_document_start_scripts() {
    let path = std::env::temp_dir().join(format!("moli-doc-start-{}.js", std::process::id()));
    std::fs::write(&path, "globalThis.__fromFile = 2;").unwrap();

    let cli = Cli::try_parse_from(normalize_args_for_compat([
        "moli",
        "fetch",
        "--document-start-script",
        "globalThis.__inline = 1;",
        "--document-start-script-file",
        path.to_str().unwrap(),
        "https://example.com",
    ]))
    .unwrap();

    let config = AppConfig::from_cli(&cli).unwrap();
    assert_eq!(
        config.document_start_scripts(),
        &[
            "globalThis.__inline = 1;".to_owned(),
            "globalThis.__fromFile = 2;".to_owned(),
        ]
    );

    let _ = std::fs::remove_file(path);
}
