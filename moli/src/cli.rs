use std::{ffi::OsString, num::NonZeroU32};

use clap::{Args, Parser, Subcommand, ValueEnum};
use regex::Regex;

const DUMP_MODES: &[&str] = &[
    "json",
    "html",
    "markdown",
    "screenshot",
    "screenshot_full",
    "pdf",
    "semantic_tree",
    "semantic_tree_text",
];

pub const DEFAULT_REDIRECT_WAIT_MS: u64 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Parser)]
#[command(
    name = "moli",
    version,
    about = "A structured-first headless browser engine for AI agents",
    subcommand_required = true,
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum Commands {
    Fetch(Box<FetchArgs>),
    Serve(Box<ServeArgs>),
}

#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct FetchArgs {
    /// Select the fetch output format. `screenshot` writes a viewport PNG,
    /// `screenshot_full` writes a full-document PNG, and `pdf` writes a
    /// paginated PDF directly to stdout; all three require layout.
    #[arg(short, long, value_enum)]
    pub dump: Option<DumpFormat>,

    #[arg(short = 'H', long = "header", value_name = "HEADER", value_parser = parse_request_header_arg)]
    pub headers: Vec<RequestHeaderArg>,

    #[arg(long)]
    pub noscript: bool,

    #[arg(long)]
    pub with_base: bool,

    #[arg(long)]
    pub with_frames: bool,

    #[arg(long)]
    pub trace_network: bool,

    #[arg(long, requires = "trace_network")]
    pub trace_matched_response_body: bool,

    #[arg(long, value_enum, value_delimiter = ',')]
    pub strip_mode: Vec<StripModeChoice>,

    #[arg(long, value_enum, default_value = "done")]
    pub wait_until: FetchWaitUntil,

    /// Milliseconds to wait for a client-side replacement navigation after an
    /// executable 4xx/5xx Document reaches the selected lifecycle stage. A
    /// value of 0 disables additional waiting but still accepts a navigation
    /// that is already pending when the stage result is inspected. This only
    /// applies to `domcontentloaded`, `load`, and `done` waits.
    #[arg(
        long,
        value_name = "MILLISECONDS",
        default_value_t = DEFAULT_REDIRECT_WAIT_MS
    )]
    pub redirect_wait_ms: u64,

    #[arg(long)]
    pub wait_selector: Option<String>,

    #[arg(long)]
    pub wait_script: Option<String>,

    #[arg(long)]
    pub wait_script_file: Option<String>,

    #[arg(long, default_value_t = 0)]
    pub delay_ms: u64,

    /// Wait for a response whose original or final URL contains this literal
    /// substring.
    #[arg(
        long,
        value_name = "SUBSTRING",
        conflicts_with = "wait_response_url_regex"
    )]
    pub wait_response_url: Option<String>,

    /// Wait for a response whose original or final URL matches this regex.
    #[arg(
        long,
        value_name = "REGEX",
        value_parser = parse_response_url_regex,
        conflicts_with = "wait_response_url"
    )]
    pub wait_response_url_regex: Option<ResponseRegexArg>,

    /// Wait for a response whose body contains this literal substring.
    #[arg(
        long,
        value_name = "SUBSTRING",
        conflicts_with = "wait_response_body_regex"
    )]
    pub wait_response_body: Option<String>,

    /// Wait for a response whose body matches this regex.
    #[arg(
        long,
        value_name = "REGEX",
        value_parser = parse_response_body_regex,
        conflicts_with = "wait_response_body"
    )]
    pub wait_response_body_regex: Option<ResponseRegexArg>,

    /// Wait for a response whose JSON field equals this literal value.
    #[arg(
        long,
        value_name = "PATH=VALUE",
        value_parser = parse_response_json_path_arg,
        conflicts_with = "wait_response_json_regex"
    )]
    pub wait_response_json: Option<ResponseJsonPathArg>,

    /// Wait for a response whose JSON scalar field's textual value matches
    /// this regex.
    #[arg(
        long,
        value_name = "PATH=REGEX",
        value_parser = parse_response_json_path_regex_arg,
        conflicts_with = "wait_response_json"
    )]
    pub wait_response_json_regex: Option<ResponseJsonPathRegexArg>,

    /// Maximum total readiness time in milliseconds. Initial and HTTP-error
    /// replacement navigations, the selected lifecycle stage, response match,
    /// selector, and script waits share one absolute deadline. Network-idle and
    /// DOM-stable return the current page with a warning when it expires.
    #[arg(short, long, alias = "wait-ms", default_value_t = 25_000)]
    pub timeout: u64,

    #[command(flatten)]
    pub common: CommonArgs,

    #[arg(value_name = "URL")]
    pub url: String,
}

impl FetchArgs {
    pub fn strip_options(&self) -> StripOptions {
        let mut strip = StripOptions::default();

        if self.noscript {
            strip.js = true;
        }

        for mode in &self.strip_mode {
            match mode {
                StripModeChoice::Js => strip.js = true,
                StripModeChoice::Ui => strip.ui = true,
                StripModeChoice::Css => strip.css = true,
                StripModeChoice::Full => {
                    strip.js = true;
                    strip.ui = true;
                    strip.css = true;
                }
            }
        }

        strip
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseJsonPathArg {
    pub path: Vec<String>,
    pub expected: String,
}

fn parse_response_json_path_arg(raw: &str) -> Result<ResponseJsonPathArg, String> {
    let (path, expected) = split_response_json_path_arg(raw, "path=value")?;
    Ok(ResponseJsonPathArg {
        path,
        expected: expected.to_owned(),
    })
}

fn split_response_json_path_arg<'a>(
    raw: &'a str,
    expected_form: &str,
) -> Result<(Vec<String>, &'a str), String> {
    let Some(separator) = raw.find('=') else {
        return Err(format!(
            "response JSON wait must be in '{expected_form}' form"
        ));
    };
    let path = raw[..separator].trim();
    if path.is_empty() {
        return Err("response JSON wait path must not be empty".to_owned());
    }
    let segments = path
        .split('.')
        .map(str::trim)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if segments.iter().any(String::is_empty) {
        return Err("response JSON wait path must not contain empty segments".to_owned());
    }
    Ok((segments, &raw[separator + 1..]))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseJsonPathRegexArg {
    pub path: Vec<String>,
    regex: ResponseRegexArg,
}

impl ResponseJsonPathRegexArg {
    pub(crate) fn regex(&self) -> &Regex {
        self.regex.regex()
    }

    pub fn pattern(&self) -> &str {
        self.regex.pattern()
    }
}

fn parse_response_json_path_regex_arg(raw: &str) -> Result<ResponseJsonPathRegexArg, String> {
    let (path, pattern) = split_response_json_path_arg(raw, "path=regex")?;
    Ok(ResponseJsonPathRegexArg {
        path,
        regex: parse_response_regex(pattern, "--wait-response-json-regex")?,
    })
}

#[derive(Debug, Clone)]
pub struct ResponseRegexArg {
    regex: Regex,
}

impl ResponseRegexArg {
    pub(crate) fn regex(&self) -> &Regex {
        &self.regex
    }

    pub fn pattern(&self) -> &str {
        self.regex.as_str()
    }
}

impl PartialEq for ResponseRegexArg {
    fn eq(&self, other: &Self) -> bool {
        self.regex.as_str() == other.regex.as_str()
    }
}

impl Eq for ResponseRegexArg {}

fn parse_response_url_regex(raw: &str) -> Result<ResponseRegexArg, String> {
    parse_response_regex(raw, "--wait-response-url-regex")
}

fn parse_response_body_regex(raw: &str) -> Result<ResponseRegexArg, String> {
    parse_response_regex(raw, "--wait-response-body-regex")
}

fn parse_response_regex(raw: &str, option: &str) -> Result<ResponseRegexArg, String> {
    Regex::new(raw)
        .map(|regex| ResponseRegexArg { regex })
        .map_err(|error| format!("invalid {option} regex: {error}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestHeaderArg {
    pub name: String,
    pub value: String,
}

fn parse_request_header_arg(raw: &str) -> Result<RequestHeaderArg, String> {
    let Some(separator) = raw.find(':') else {
        return Err("header must be in 'Name: Value' form".to_owned());
    };

    let name = raw[..separator].trim();
    if name.is_empty() {
        return Err("header name must not be empty".to_owned());
    }

    let value = raw[separator + 1..].trim_start_matches([' ', '\t']);
    Ok(RequestHeaderArg {
        name: name.to_owned(),
        value: value.to_owned(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct ServeArgs {
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    #[arg(short, long, default_value_t = 9222)]
    pub port: u16,

    #[arg(short, long, default_value_t = 10)]
    pub timeout: u32,

    #[arg(long, default_value_t = 16)]
    pub cdp_max_connections: u16,

    #[arg(long, default_value_t = 128)]
    pub cdp_max_pending_connections: u16,

    #[command(flatten)]
    pub common: CommonArgs,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Args)]
pub struct CommonArgs {
    #[arg(long)]
    pub insecure_disable_tls_host_verification: bool,

    /// Refuse a `fetch` whose URL the origin's `/robots.txt` disallows for the
    /// configured user agent. An unreachable `robots.txt` (5xx or a transport
    /// failure) disallows the whole origin, per RFC 9309. Subresources and
    /// `serve` navigations are not checked.
    #[arg(long)]
    pub obey_robots: bool,

    #[arg(long)]
    pub http_proxy: Option<String>,

    #[arg(long)]
    pub http_no_proxy: Option<String>,

    #[arg(long = "http-host-resolve", value_name = "HOST:PORT:ADDR")]
    pub http_host_resolve: Vec<String>,

    #[arg(long)]
    pub proxy_bearer_token: Option<String>,

    #[arg(long)]
    pub http_max_concurrent: Option<NonZeroU32>,

    /// Limit active fetch-runtime transfers per origin.
    ///
    /// This is a scheduler limit, not libcurl's per-host connection-pool cap.
    /// Use `--http-max-host-connections` to change the HTTP/1-style transport
    /// connection cap.
    #[arg(long)]
    pub http_max_host_open: Option<NonZeroU32>,

    /// Limit transport connections per host/group.
    ///
    /// Defaults to 6 to match Chromium's normal HTTP/1 socket-pool shape. This
    /// does not limit HTTP/2 streams; use `--http2-max-concurrent-streams` for
    /// that.
    #[arg(long)]
    pub http_max_host_connections: Option<u8>,

    /// Limit total transport connections across all hosts.
    #[arg(long)]
    pub http_max_total_connections: Option<u16>,

    /// Limit concurrent HTTP/2 streams.
    #[arg(long)]
    pub http2_max_concurrent_streams: Option<u16>,

    /// Override the HTTP connect timeout in milliseconds for every request.
    ///
    /// Without an override, top-level navigations retain the transport default
    /// and browser subresources use a 10-second connect timeout.
    #[arg(long)]
    pub http_connect_timeout: Option<u32>,

    #[arg(long)]
    pub http_timeout: Option<u32>,

    #[arg(long)]
    pub http_max_response_size: Option<usize>,

    #[arg(long)]
    pub http_cache_dir: Option<String>,

    #[arg(short = 'P', long)]
    pub profile_dir: Option<String>,

    #[arg(long)]
    pub image: bool,

    #[arg(long)]
    pub font: bool,

    #[arg(long)]
    pub audio: bool,

    #[arg(long)]
    pub video: bool,

    #[arg(long)]
    pub media: bool,

    #[arg(long)]
    pub text_track: bool,

    /// Fetch every optional image, font, audio, video, media, and text-track
    /// resource family.
    #[arg(
        short,
        long,
        env = "MOLI_RESOURCE",
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub resource: bool,

    #[arg(long)]
    pub disable_subframes: bool,

    /// Enable the real on-demand layout renderer and screenshot surfaces.
    ///
    /// Without this flag Moli keeps deterministic compatibility
    /// geometry and does not construct layout or paint output.
    #[arg(
        short,
        long,
        env = "MOLI_LAYOUT",
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub layout: bool,

    #[arg(short, long = "cookie-file")]
    pub cookie_file: Vec<String>,

    #[arg(long)]
    pub document_start_script: Vec<String>,

    #[arg(long)]
    pub document_start_script_file: Vec<String>,

    #[arg(
        long,
        env = "MOLI_BLOCK_PRIVATE_NETWORKS",
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub block_private_networks: bool,

    #[arg(long)]
    pub block_cidrs: Option<String>,

    #[arg(short = 'L', long, value_enum)]
    pub log_level: Option<LogLevel>,

    #[arg(long, value_enum)]
    pub log_format: Option<LogFormat>,

    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    pub log_filter_scopes: Option<String>,

    #[arg(short = 'A', long)]
    pub user_agent: Option<String>,

    #[arg(long)]
    pub user_agent_suffix: Option<String>,

    /// Unencrypted PKCS#8 Ed25519 private key used for Web Bot Auth signatures.
    #[arg(long, value_name = "PATH", requires = "web_bot_auth_domain")]
    pub web_bot_auth_key_file: Option<String>,

    /// Assert the RFC 7638 JWK thumbprint derived from the Web Bot Auth key.
    #[arg(long, value_name = "THUMBPRINT", requires = "web_bot_auth_key_file")]
    pub web_bot_auth_keyid: Option<String>,

    /// Operator domain publishing /.well-known/http-message-signatures-directory.
    #[arg(long, value_name = "DOMAIN", requires = "web_bot_auth_key_file")]
    pub web_bot_auth_domain: Option<String>,

    /// Signature-Agent wire format. Cloudflare compatibility is the default.
    #[arg(
        long,
        value_enum,
        default_value = "cloudflare",
        requires = "web_bot_auth_key_file"
    )]
    pub web_bot_auth_profile: WebBotAuthProfileChoice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum WebBotAuthProfileChoice {
    #[default]
    Cloudflare,
    #[value(name = "ietf-01")]
    IetfDraft01,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StripOptions {
    pub js: bool,
    pub ui: bool,
    pub css: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl LogLevel {
    pub fn as_tracing_filter(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error | Self::Fatal => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum LogFormat {
    Pretty,
    Logfmt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum DumpFormat {
    // Stable machine-readable output for scrapling-style integrations.
    Json,
    Html,
    Markdown,
    Screenshot,
    ScreenshotFull,
    Pdf,
    SemanticTree,
    SemanticTreeText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum StripModeChoice {
    Js,
    Ui,
    Css,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum FetchWaitUntil {
    #[value(alias = "dcl")]
    DomContentLoaded,
    Load,
    NetworkIdle,
    DomStable,
    Done,
}

pub fn normalize_args_for_compat<I, T>(itr: I) -> Vec<OsString>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let mut args: Vec<OsString> = itr.into_iter().map(Into::into).collect();

    if args.is_empty() {
        args.push(OsString::from("moli"));
    }

    if let Some(inferred_command) = infer_command(args.get(1)) {
        args.insert(1, OsString::from(inferred_command));
    }

    normalize_dump_flag(&mut args);
    args
}

fn infer_command(next: Option<&OsString>) -> Option<&'static str> {
    let Some(next) = next else {
        return Some("serve");
    };
    let next = next.to_string_lossy();

    if next.starts_with("http://") || next.starts_with("https://") {
        return Some("fetch");
    }

    None
}

fn normalize_dump_flag(args: &mut Vec<OsString>) {
    let Some(index) = args.iter().position(|arg| arg == "--dump" || arg == "-d") else {
        return;
    };

    let next = args
        .get(index + 1)
        .map(|arg| arg.to_string_lossy().into_owned());
    let next_is_valid_dump_mode = next
        .as_deref()
        .is_some_and(|candidate| DUMP_MODES.contains(&candidate));

    if !next_is_valid_dump_mode {
        args.insert(index + 1, OsString::from("html"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_uses_the_public_product_description() {
        let help = Cli::try_parse_from(["moli", "--help"])
            .unwrap_err()
            .to_string();

        assert!(help.contains("A structured-first headless browser engine for AI agents"));
        assert!(!help.contains("scaffold"));
    }

    #[test]
    fn parses_delay_ms_with_explicit_fetch_command() {
        let args = normalize_args_for_compat([
            "moli",
            "fetch",
            "--delay-ms",
            "250",
            "https://example.test/",
        ]);
        let cli = Cli::parse_from(args);

        match cli.command {
            Commands::Fetch(args) => assert_eq!(args.delay_ms, 250),
            other => panic!("expected fetch command, got {other:?}"),
        }
    }

    #[test]
    fn parses_redirect_wait_ms_with_explicit_fetch_command() {
        let args = normalize_args_for_compat([
            "moli",
            "fetch",
            "--redirect-wait-ms=1500",
            "https://example.test/",
        ]);
        let cli = Cli::parse_from(args);

        match cli.command {
            Commands::Fetch(args) => assert_eq!(args.redirect_wait_ms, 1_500),
            other => panic!("expected fetch command, got {other:?}"),
        }
    }

    #[test]
    fn fetch_help_exposes_only_redirect_wait_ms() {
        let help = Cli::try_parse_from(["moli", "fetch", "--help"])
            .unwrap_err()
            .to_string();

        assert!(help.contains("--redirect-wait-ms <MILLISECONDS>"));
        assert!(!help.contains("--redirect-time"));
        assert!(help.contains("Maximum total readiness time in milliseconds"));
        assert!(help.contains("response match, selector, and script waits share one absolute"));
    }

    #[test]
    fn fetch_help_labels_literal_and_regex_response_waits() {
        let help = Cli::try_parse_from(["moli", "fetch", "--help"])
            .unwrap_err()
            .to_string();

        assert!(help.contains("--wait-response-url <SUBSTRING>"));
        assert!(help.contains("--wait-response-url-regex <REGEX>"));
        assert!(help.contains("--wait-response-body <SUBSTRING>"));
        assert!(help.contains("--wait-response-body-regex <REGEX>"));
        assert!(help.contains("--wait-response-json <PATH=VALUE>"));
        assert!(help.contains("--wait-response-json-regex <PATH=REGEX>"));
    }

    #[test]
    fn parses_port_equals_with_explicit_serve_command() {
        let args = normalize_args_for_compat(["moli", "serve", "--port=0"]);
        let cli = Cli::parse_from(args);

        match cli.command {
            Commands::Serve(args) => assert_eq!(args.port, 0),
            other => panic!("expected serve command, got {other:?}"),
        }
    }

    #[test]
    fn short_flags_do_not_collide() {
        // clap panics on a duplicate short within one command. `CommonArgs` is
        // flattened into both subcommands, so this is the only check that
        // covers each union rather than each struct in isolation.
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn help_is_still_reachable_as_dash_h() {
        for command in [
            ["moli", "fetch", "-h"].as_slice(),
            ["moli", "serve", "-h"].as_slice(),
        ] {
            let rendered = Cli::try_parse_from(command.iter().copied())
                .expect_err("-h must render help rather than parse")
                .to_string();
            assert!(rendered.contains("Usage:"), "{command:?} -> {rendered}");
        }
    }

    #[test]
    fn short_flags_match_their_long_spelling() {
        let long = Cli::parse_from([
            "moli",
            "fetch",
            "--dump",
            "markdown",
            "--wait-until",
            "load",
            "--wait-selector",
            "main",
            "--timeout",
            "5000",
            "--noscript",
            "--layout",
            "--image",
            "--font",
            "--cookie-file",
            "jar.txt",
            "--profile-dir",
            "/tmp/p",
            "--user-agent",
            "Bot/1.0",
            "--log-level",
            "debug",
            "https://example.test/",
        ]);
        let short = Cli::parse_from([
            "moli",
            "fetch",
            "-d",
            "markdown",
            "--wait-until",
            "load",
            "--wait-selector",
            "main",
            "-t",
            "5000",
            "--noscript",
            "-l",
            "--image",
            "--font",
            "-c",
            "jar.txt",
            "-P",
            "/tmp/p",
            "-A",
            "Bot/1.0",
            "-L",
            "debug",
            "https://example.test/",
        ]);

        assert_eq!(long, short);
    }

    #[test]
    fn serve_short_flags_match_their_long_spelling() {
        let long = Cli::parse_from([
            "moli",
            "serve",
            "--port",
            "9333",
            "--timeout",
            "30",
            "--layout",
            "--resource",
        ]);
        let short = Cli::parse_from(["moli", "serve", "-p", "9333", "-t", "30", "-l", "-r"]);

        assert_eq!(long, short);
    }

    #[test]
    fn boolean_short_flags_can_be_grouped() {
        // The shape the feature request asked for: `--layout --resource` as `-lr`.
        let grouped = Cli::parse_from(["moli", "serve", "-lr"]);
        let separate = Cli::parse_from(["moli", "serve", "--layout", "--resource"]);
        assert_eq!(grouped, separate);

        match grouped.command {
            Commands::Serve(args) => {
                assert!(args.common.layout);
                assert!(args.common.resource);
            }
            other => panic!("expected serve command, got {other:?}"),
        }
    }

    #[test]
    fn removed_short_flags_are_rejected() {
        for arguments in [
            ["moli", "fetch", "-n", "https://example.test/"].as_slice(),
            ["moli", "fetch", "-w", "load", "https://example.test/"].as_slice(),
            ["moli", "fetch", "-s", "main", "https://example.test/"].as_slice(),
            ["moli", "fetch", "-i", "https://example.test/"].as_slice(),
            ["moli", "fetch", "-f", "https://example.test/"].as_slice(),
            ["moli", "fetch", "-a", "https://example.test/"].as_slice(),
            ["moli", "fetch", "-v", "https://example.test/"].as_slice(),
            ["moli", "fetch", "-m", "https://example.test/"].as_slice(),
            ["moli", "fetch", "-T", "https://example.test/"].as_slice(),
        ] {
            assert!(
                Cli::try_parse_from(arguments.iter().copied()).is_err(),
                "removed short flag parsed successfully: {arguments:?}"
            );
        }
    }

    #[test]
    fn short_dump_defaults_its_value_with_explicit_fetch_command() {
        let args = normalize_args_for_compat(["moli", "fetch", "-d", "https://example.test/"]);
        let cli = Cli::parse_from(args);

        match cli.command {
            // A bare `--dump` defaults to html; `-d` must do the same.
            Commands::Fetch(args) => assert_eq!(args.dump, Some(DumpFormat::Html)),
            other => panic!("expected fetch command, got {other:?}"),
        }
    }

    #[test]
    fn flags_do_not_infer_subcommands() {
        for arguments in [
            ["moli", "--dump", "html", "https://example.test/"].as_slice(),
            ["moli", "-d", "html", "https://example.test/"].as_slice(),
            ["moli", "--port=0"].as_slice(),
            ["moli", "-p", "0"].as_slice(),
            ["moli", "--layout"].as_slice(),
        ] {
            let error = Cli::try_parse_from(normalize_args_for_compat(arguments.iter().copied()))
                .expect_err("a flag must not infer a subcommand");
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::UnknownArgument,
                "unexpected parse result for {arguments:?}"
            );
        }
    }
}
