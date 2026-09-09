//! Reassemble validated native chunks and enforce browser message policy.
use moli_curl::websocket::{WsFlags, WsFrame};

const MAX_FRAME_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MESSAGE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Assembler {
    message: Vec<u8>,
    control: Vec<u8>,
}

pub(super) enum Received {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong,
    Close {
        code: u16,
        reason: String,
        payload: Vec<u8>,
    },
}

impl Assembler {
    pub fn push(&mut self, data: Vec<u8>, meta: WsFrame) -> Result<Option<Received>, String> {
        let flags = meta.flags();
        let kind = WsFlags::from_bits(flags.bits() & !WsFlags::CONT.bits());
        let control = matches!(kind, WsFlags::CLOSE | WsFlags::PING | WsFlags::PONG);
        // Native Chunk metadata starts each frame at zero and retains the data
        // type on continuations. Check our memory limits before buffering it;
        // libcurl already validates framing and the nonnegative 63-bit length.
        if meta.offset() == 0 {
            let size = meta.len() as u64 + meta.bytes_left();
            if size > MAX_FRAME_BYTES {
                return Err("WebSocket frame exceeds size limit".to_owned());
            }
            if !control && self.message.len() as u64 + size > MAX_MESSAGE_BYTES {
                return Err("WebSocket message exceeds size limit".to_owned());
            }
        }
        if control {
            self.control.extend_from_slice(&data);
        } else {
            self.message.extend_from_slice(&data);
        }
        if meta.bytes_left() != 0 {
            return Ok(None);
        }
        if control {
            let payload = std::mem::take(&mut self.control);
            return Ok(Some(match kind {
                WsFlags::PING => Received::Ping(payload),
                WsFlags::PONG => Received::Pong,
                WsFlags::CLOSE => {
                    let (code, reason) = parse_close(&payload)?;
                    Received::Close {
                        code,
                        reason,
                        payload,
                    }
                }
                _ => unreachable!(),
            }));
        }
        if flags.contains(WsFlags::CONT) {
            return Ok(None);
        }
        let message = std::mem::take(&mut self.message);
        Ok(Some(if kind == WsFlags::TEXT {
            Received::Text(
                String::from_utf8(message).map_err(|_| "WebSocket text is not valid UTF-8")?,
            )
        } else {
            Received::Binary(message)
        }))
    }
}

pub(super) fn parse_close(payload: &[u8]) -> Result<(u16, String), String> {
    if payload.is_empty() {
        return Ok((1005, String::new()));
    }
    if payload.len() < 2 || payload.len() > 125 {
        return Err("WebSocket close payload has invalid length".to_owned());
    }
    let code = u16::from_be_bytes([payload[0], payload[1]]);
    if !matches!(code, 1000..=1003 | 1007..=1014 | 3000..=4999) {
        return Err(format!("WebSocket received invalid close code {code}"));
    }
    let reason = std::str::from_utf8(&payload[2..])
        .map_err(|_| "WebSocket close reason is not valid UTF-8")?
        .to_owned();
    Ok((code, reason))
}

pub(super) fn close_payload(code: Option<u16>, reason: String) -> Result<Vec<u8>, String> {
    let Some(code) = code else {
        return Ok(Vec::new());
    };
    let mut payload = code.to_be_bytes().to_vec();
    payload.extend_from_slice(reason.as_bytes());
    parse_close(&payload)?;
    Ok(payload)
}
