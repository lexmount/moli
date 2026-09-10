#[derive(Debug, Clone)]
pub struct ConnectOptions {
    /// Native runtime for this network scope. None selects the standalone
    /// process runtime. A closed provided connector fails without fallback.
    pub connector: Option<moli_curl::websocket::CurlWebSocketConnector>,
    pub origin: String,
    pub user_agent: String,
    pub extra_headers: Vec<(String, String)>,
    pub http_proxy: Option<String>,
    pub http_no_proxy: Option<String>,
    pub proxy_bearer_token: Option<String>,
    pub tls: moli_curl::CurlTlsConfig,
    pub cookie_header: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOpcode {
    Text,
    Binary,
}

#[derive(Debug, Clone)]
pub enum Event {
    HandshakeResponse {
        socket_id: u64,
        protocol: String,
        extensions: String,
        request_headers: Vec<(String, String)>,
        response_status: u16,
        response_headers: Vec<(String, String)>,
    },
    Open {
        socket_id: u64,
        protocol: String,
        extensions: String,
        request_headers: Vec<(String, String)>,
        response_status: u16,
        response_headers: Vec<(String, String)>,
    },
    TextMessage {
        socket_id: u64,
        data: String,
    },
    BinaryMessage {
        socket_id: u64,
        data: Vec<u8>,
    },
    /// One complete outgoing message, including an empty message.
    /// Drives browser byte accounting, stream writes and CDP message records.
    SendCompleted {
        socket_id: u64,
        opcode: FrameOpcode,
        payload_length: usize,
    },
    Error {
        socket_id: u64,
        message: String,
    },
    Closing {
        socket_id: u64,
    },
    Close {
        socket_id: u64,
        code: u16,
        reason: String,
        was_clean: bool,
    },
}

impl Event {
    pub fn socket_id(&self) -> u64 {
        match self {
            Self::HandshakeResponse { socket_id, .. }
            | Self::Open { socket_id, .. }
            | Self::TextMessage { socket_id, .. }
            | Self::BinaryMessage { socket_id, .. }
            | Self::SendCompleted { socket_id, .. }
            | Self::Error { socket_id, .. }
            | Self::Closing { socket_id }
            | Self::Close { socket_id, .. } => *socket_id,
        }
    }
}

#[derive(Debug)]
pub(crate) enum Command {
    SendText(String),
    SendBinary(Vec<u8>),
    ReceiveText(String),
    ReceiveBinary(Vec<u8>),
    Fail(String),
    ServerClose { code: Option<u16>, reason: String },
    Close { code: Option<u16>, reason: String },
}
