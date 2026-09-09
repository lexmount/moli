//! Reassemble native chunks and validate the browser's message boundary.
use moli_curl::websocket::{WsFlags, WsFrame};

const MAX_FRAME_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Assembler {
    frame: Option<FrameProgress>,
    message_kind: Option<WsFlags>,
    message: Vec<u8>,
    control: Vec<u8>,
}

struct FrameProgress {
    flags: WsFlags,
    size: u64,
    offset: u64,
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
        if !control && kind != WsFlags::TEXT && kind != WsFlags::BINARY {
            return Err("WebSocket received invalid frame flags".to_owned());
        }
        let end = meta
            .offset()
            .checked_add(data.len() as u64)
            .ok_or("WebSocket frame length overflow")?;
        let size = end
            .checked_add(meta.bytes_left())
            .ok_or("WebSocket frame length overflow")?;
        if size > MAX_FRAME_BYTES || data.len() != meta.len() {
            return Err("WebSocket frame exceeds size limit or has invalid metadata".to_owned());
        }
        if control && (size > 125 || flags.contains(WsFlags::CONT)) {
            return Err("WebSocket received invalid control frame".to_owned());
        }
        if self.frame.is_none() {
            if meta.offset() != 0 {
                return Err("WebSocket frame starts at a nonzero offset".to_owned());
            }
            self.frame = Some(FrameProgress {
                flags,
                size,
                offset: 0,
            });
            if !control {
                if self.message_kind.is_some_and(|previous| previous != kind) {
                    return Err("WebSocket fragmented message changed type".to_owned());
                }
                self.message_kind = Some(kind);
                if self
                    .message
                    .len()
                    .checked_add(size as usize)
                    .is_none_or(|len| len > MAX_MESSAGE_BYTES)
                {
                    return Err("WebSocket message exceeds size limit".to_owned());
                }
            }
        }
        let frame = self.frame.as_mut().expect("frame started");
        if frame.offset != meta.offset() || frame.size != size || frame.flags != flags {
            return Err("WebSocket received inconsistent frame chunks".to_owned());
        }
        frame.offset = end;
        if control {
            self.control.extend_from_slice(&data);
        } else {
            self.message.extend_from_slice(&data);
        }
        if meta.bytes_left() != 0 {
            return Ok(None);
        }
        self.frame = None;
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
        self.message_kind = None;
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
