#[derive(Debug, Clone)]
pub struct ConnectOptions {
    pub origin: String,
    pub user_agent: String,
    pub extra_headers: Vec<(String, String)>,
    pub http_proxy: Option<String>,
    pub http_no_proxy: Option<String>,
    pub proxy_bearer_token: Option<String>,
    pub tls_verify_host: bool,
    pub cookie_header: Option<String>,
    pub pause_after_handshake: bool,
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
    FrameSent {
        socket_id: u64,
        opcode: FrameOpcode,
        payload_length: usize,
    },
    BufferedAmountConsumed {
        socket_id: u64,
        amount: usize,
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
            | Self::FrameSent { socket_id, .. }
            | Self::BufferedAmountConsumed { socket_id, .. }
            | Self::Error { socket_id, .. }
            | Self::Closing { socket_id }
            | Self::Close { socket_id, .. } => *socket_id,
        }
    }
}

#[derive(Debug)]
pub enum Command {
    SendText(String),
    SendBinary(Vec<u8>),
    ReceiveText(String),
    ReceiveBinary(Vec<u8>),
    ContinueOpen {
        response_status: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    },
    FailOpen(String),
    ServerClose {
        code: Option<u16>,
        reason: String,
    },
    Close {
        code: Option<u16>,
        reason: String,
    },
}

/// Compatibility name for the connection's producer capability.
pub type CommandSender = crate::ConnectionHandle;
