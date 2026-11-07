// WebTransport shares with HTTP/3, so we can't start at 0 or use the full VarInt.
const ERROR_FIRST: u64 = 0x52e4a40fa8db;
const ERROR_LAST: u64 = 0x52e5ac983162;

pub const fn error_from_http3(code: u64) -> Option<u32> {
    if code < ERROR_FIRST || code > ERROR_LAST {
        return None;
    }

    let code = code - ERROR_FIRST;
    let code = code - code / 0x1f;

    Some(code as u32)
}

pub const fn error_to_http3(code: u32) -> u64 {
    ERROR_FIRST + code as u64 + code as u64 / 0x1e
}
