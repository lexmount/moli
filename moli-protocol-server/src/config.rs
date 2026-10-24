pub(crate) const DEFAULT_SCREENCAST_INTERVAL_MS: u32 = 1000;

/// Network and connection settings for the automation protocol server.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub timeout_secs: u32,
    /// Base screencast interval in milliseconds; must be positive.
    pub screencast_interval_ms: u32,
    /// One MP4 recording of the startup main Page; requires real layout.
    pub screencast_video: Option<std::path::PathBuf>,
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
            screencast_interval_ms: DEFAULT_SCREENCAST_INTERVAL_MS,
            screencast_video: None,
        }
    }
}
