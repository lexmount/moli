pub(crate) const DEFAULT_SCREENCAST_INTERVAL_MS: u32 = 1000;

/// Network and connection settings for the automation protocol server.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub timeout_secs: u32,
    /// Additional allowed origins, one per entry; `*` allows all origins.
    /// Requests without Origin and requests from the bound server's own HTTP
    /// origin are allowed by default. Other origins require this allowlist.
    pub remote_allow_origins: Vec<String>,
    /// Base screencast interval in milliseconds; must be positive.
    pub screencast_interval_ms: u32,
}

impl ServerConfig {
    pub fn bind_target(&self) -> (&str, u16) {
        (&self.host, self.port)
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_owned(),
            port: 9222,
            timeout_secs: 10,
            remote_allow_origins: Vec::new(),
            screencast_interval_ms: DEFAULT_SCREENCAST_INTERVAL_MS,
        }
    }
}
