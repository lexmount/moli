use anyhow::{Context, Result, bail};
use cidr::AnyIpCidr;
use moli_browser_profile::BrowserProfilePaths;
use moli_core::{
    LayoutPolicy, OptionalResourceFetchMask,
    page::{SubresourceJsonPathEquals, SubresourceJsonPathRegex, SubresourceResponseWaitCriteria},
    runtime::BrowserConfig,
};
use moli_fetch::{FetchConfig, WebBotAuthProfile, WebBotAuthSigner};
use std::path::PathBuf;
use std::str::FromStr;

use crate::cli::{
    Cli, Commands, CommonArgs, DumpFormat, LogFormat, StripOptions, WebBotAuthProfileChoice,
};
use crate::network_trace::NetworkTraceConfigSummary;

pub use moli_protocol_server::ServerConfig;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub log_filter: String,
    pub browser: BrowserConfig,
    pub server: ServerConfig,
    pub fetch: FetchCommandConfig,
}

impl AppConfig {
    pub fn from_cli(cli: &Cli) -> Result<Self> {
        let mut config = Self::default();

        match &cli.command {
            Commands::Fetch(args) => {
                apply_common_args(&mut config, &args.common)?;
                config.fetch.request_headers = args
                    .headers
                    .iter()
                    .map(|header| (header.name.clone(), header.value.clone()))
                    .collect();
                config.fetch.dump_mode = args.dump;
                config.fetch.strip = args.strip_options();
                config.fetch.with_base = args.with_base;
                config.fetch.with_frames = args.with_frames;
                config.fetch.trace_network = args.trace_network;
                config.fetch.trace_matched_response_body = args.trace_matched_response_body;
                config.fetch.network_trace_config =
                    Some(NetworkTraceConfigSummary::from(config.browser.fetch()));
                let response_wait = response_wait_criteria_from_args(args);
                if !response_wait.is_empty() {
                    config.fetch.response_wait = Some(response_wait);
                }
            }
            Commands::Serve(args) => {
                apply_common_args(&mut config, &args.common)?;
                config.server.host = args.host.clone();
                config.server.port = args.port;
                config.server.timeout_secs = args.timeout;
                config.server.cdp_max_connections = args.cdp_max_connections;
                config.server.cdp_max_pending_connections = args.cdp_max_pending_connections;
            }
        }

        Ok(config)
    }

    pub fn document_start_scripts(&self) -> &[String] {
        self.browser.document_start_scripts()
    }

    pub fn add_document_start_script(&mut self, source: impl Into<String>) {
        self.browser.add_document_start_script(source);
    }

    pub fn with_document_start_script(mut self, source: impl Into<String>) -> Self {
        self.add_document_start_script(source);
        self
    }
}

fn apply_common_args(config: &mut AppConfig, common: &CommonArgs) -> Result<()> {
    if let Some(log_level) = common.log_level {
        config.log_filter = log_level.as_tracing_filter().to_owned();
    }

    if let Some(user_agent) = &common.user_agent {
        config
            .browser
            .fetch_mut()
            .set_user_agent(user_agent.clone());
    } else if let Some(user_agent_suffix) = &common.user_agent_suffix {
        config
            .browser
            .fetch_mut()
            .set_user_agent_suffix(user_agent_suffix);
    }

    if let Some(http_timeout) = common.http_timeout {
        config
            .browser
            .fetch_mut()
            .set_request_timeout_ms(u64::from(http_timeout));
    }

    config
        .browser
        .fetch_mut()
        .set_connect_timeout_ms(common.http_connect_timeout.map(u64::from));
    config
        .browser
        .fetch_mut()
        .set_obey_robots(common.obey_robots);
    config
        .browser
        .fetch_mut()
        .set_proxy_options(common.http_proxy.clone(), common.proxy_bearer_token.clone());
    config
        .browser
        .fetch_mut()
        .set_http_no_proxy(common.http_no_proxy.clone());
    validate_http_host_resolve_entries(&common.http_host_resolve)?;
    config
        .browser
        .fetch_mut()
        .set_http_host_resolve(common.http_host_resolve.clone());
    config.browser.fetch_mut().set_connection_limits(
        common.http_max_concurrent,
        common.http_max_host_open,
        common.http_max_response_size,
    );
    config.browser.fetch_mut().set_transport_connection_limits(
        common.http_max_host_connections,
        common.http_max_total_connections,
        common.http2_max_concurrent_streams,
    );
    config
        .browser
        .fetch_mut()
        .set_http_cache_dir(common.http_cache_dir.clone());
    if let Some(profile_dir) = &common.profile_dir {
        config
            .browser
            .set_profile_dir(Some(PathBuf::from(profile_dir)));
        if common.http_cache_dir.is_none() {
            let profile = BrowserProfilePaths::new(profile_dir);
            config
                .browser
                .fetch_mut()
                .set_http_cache_dir(Some(profile.http_cache_root.display().to_string()));
        }
    }
    let mut optional_resource_fetch_mask = if common.resource {
        OptionalResourceFetchMask::ALL
    } else {
        OptionalResourceFetchMask::NONE
    };
    for (enabled, resource) in [
        (common.image, OptionalResourceFetchMask::IMAGE),
        (common.font, OptionalResourceFetchMask::FONT),
        (common.audio, OptionalResourceFetchMask::AUDIO),
        (common.video, OptionalResourceFetchMask::VIDEO),
        (common.media, OptionalResourceFetchMask::MEDIA),
        (common.text_track, OptionalResourceFetchMask::TEXT_TRACK),
    ] {
        if enabled {
            optional_resource_fetch_mask |= resource;
        }
    }
    config
        .browser
        .set_optional_resource_fetch_mask(optional_resource_fetch_mask);
    config
        .browser
        .set_subframe_loading_enabled(!common.disable_subframes);
    config.browser.set_layout_policy(if common.layout {
        LayoutPolicy::OnDemand
    } else {
        LayoutPolicy::Mock
    });
    config.fetch.cookie_files = common.cookie_file.clone();
    config.browser.fetch_mut().set_network_blocking(
        common.block_private_networks,
        parse_block_cidrs(common.block_cidrs.as_deref()),
    );
    config
        .browser
        .fetch_mut()
        .set_tls_verify_host(!common.insecure_disable_tls_host_verification);
    configure_web_bot_auth(config.browser.fetch_mut(), common)?;

    for source in &common.document_start_script {
        config.add_document_start_script(source.clone());
    }

    for path in &common.document_start_script_file {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read document-start script file `{path}`"))?;
        config.add_document_start_script(source);
    }

    config.fetch.log_format = common.log_format;
    config.fetch.log_filter_scopes = common.log_filter_scopes.clone();
    Ok(())
}

fn configure_web_bot_auth(fetch: &mut FetchConfig, common: &CommonArgs) -> Result<()> {
    let (key_file, domain) = match (
        common.web_bot_auth_key_file.as_deref(),
        common.web_bot_auth_domain.as_deref(),
    ) {
        (None, None) => {
            if common.web_bot_auth_keyid.is_some() {
                bail!("--web-bot-auth-keyid requires --web-bot-auth-key-file");
            }
            return Ok(());
        }
        (Some(_), None) => bail!("--web-bot-auth-key-file requires --web-bot-auth-domain"),
        (None, Some(_)) => bail!("--web-bot-auth-domain requires --web-bot-auth-key-file"),
        (Some(key_file), Some(domain)) => (key_file, domain),
    };
    let private_key_pem = std::fs::read(key_file)
        .with_context(|| format!("failed to read Web Bot Auth private key `{key_file}`"))?;
    let profile = match common.web_bot_auth_profile {
        WebBotAuthProfileChoice::Cloudflare => WebBotAuthProfile::Cloudflare,
        WebBotAuthProfileChoice::IetfDraft01 => WebBotAuthProfile::IetfDraft01,
    };
    let signer = WebBotAuthSigner::from_pem(
        &private_key_pem,
        domain,
        common.web_bot_auth_keyid.as_deref(),
        profile,
    )?;
    fetch.set_web_bot_auth(Some(signer));
    Ok(())
}

fn validate_http_host_resolve_entries(entries: &[String]) -> Result<()> {
    for entry in entries {
        validate_http_host_resolve_entry(entry)?;
    }
    Ok(())
}

fn validate_http_host_resolve_entry(entry: &str) -> Result<()> {
    let mut parts = entry.splitn(3, ':');
    let host = parts.next().unwrap_or_default();
    let port = parts.next().unwrap_or_default();
    let address = parts.next().unwrap_or_default();

    if host.is_empty() || port.is_empty() || address.is_empty() {
        bail!("--http-host-resolve must be in HOST:PORT:ADDR form");
    }
    port.parse::<u16>()
        .with_context(|| format!("invalid --http-host-resolve port in `{entry}`"))?;
    Ok(())
}

fn parse_block_cidrs(raw: Option<&str>) -> Vec<AnyIpCidr> {
    raw.into_iter()
        .flat_map(|value| value.split(','))
        .filter_map(|item| {
            let trimmed = item.trim();
            (!trimmed.is_empty())
                .then(|| AnyIpCidr::from_str(trimmed).ok())
                .flatten()
        })
        .collect()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            log_filter: "info".to_owned(),
            browser: BrowserConfig::default(),
            server: ServerConfig::default(),
            fetch: FetchCommandConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct FetchCommandConfig {
    pub dump_mode: Option<DumpFormat>,
    pub strip: StripOptions,
    pub with_base: bool,
    pub with_frames: bool,
    pub trace_network: bool,
    pub trace_matched_response_body: bool,
    pub(crate) network_trace_config: Option<NetworkTraceConfigSummary>,
    pub response_wait: Option<SubresourceResponseWaitCriteria>,
    pub cookie_files: Vec<String>,
    // CLI request headers only apply to the top-level fetch command.
    pub request_headers: Vec<(String, String)>,
    pub log_format: Option<LogFormat>,
    pub log_filter_scopes: Option<String>,
}

pub fn response_wait_criteria_from_args(
    args: &crate::cli::FetchArgs,
) -> SubresourceResponseWaitCriteria {
    SubresourceResponseWaitCriteria {
        url_contains: args.wait_response_url.clone(),
        url_regex: args
            .wait_response_url_regex
            .as_ref()
            .map(|arg| arg.regex().clone()),
        body_contains: args.wait_response_body.clone(),
        body_regex: args
            .wait_response_body_regex
            .as_ref()
            .map(|arg| arg.regex().clone()),
        json_path_equals: args
            .wait_response_json
            .as_ref()
            .map(|json| SubresourceJsonPathEquals {
                path: json.path.clone(),
                expected: json.expected.clone(),
            }),
        json_path_regex: args.wait_response_json_regex.as_ref().map(|json| {
            SubresourceJsonPathRegex {
                path: json.path.clone(),
                regex: json.regex().clone(),
            }
        }),
    }
}
