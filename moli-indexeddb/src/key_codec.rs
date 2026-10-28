//! Version 1 binary keys for SQLite. Ordered bytes define identity; value
//! bytes retain the original representation, including the sign of zero.

use crate::{IndexedDbError, Key, MAX_KEY_DEPTH};

const NUMBER: u8 = 0x10;
const DATE: u8 = 0x20;
const STRING: u8 = 0x30;
const BINARY: u8 = 0x40;
const ARRAY: u8 = 0x50;
const SIGN_BIT: u64 = 1 << 63;
const MAX_DATE: i64 = 8_640_000_000_000_000;

#[derive(Clone, Copy)]
enum Encoding {
    Ordered,
    Value,
}

impl Key {
    /// Encode a comparison key. Byte ordering and equality match Key::cmp.
    /// The two representations of zero have one canonical ordered encoding.
    pub fn encode_ordered(&self) -> Result<Vec<u8>, IndexedDbError> {
        encode(self, Encoding::Ordered)
    }

    /// Decode a canonical ordered key, rejecting malformed or trailing bytes.
    pub fn decode_ordered(bytes: &[u8]) -> Result<Self, IndexedDbError> {
        decode(bytes, Encoding::Ordered)
    }

    /// Encode a stored key without losing the original Number bit pattern.
    pub fn encode_value(&self) -> Result<Vec<u8>, IndexedDbError> {
        encode(self, Encoding::Value)
    }

    /// Decode the lossless representation stored alongside an ordered key.
    pub fn decode_value(bytes: &[u8]) -> Result<Self, IndexedDbError> {
        decode(bytes, Encoding::Value)
    }
}

fn encode(key: &Key, encoding: Encoding) -> Result<Vec<u8>, IndexedDbError> {
    key.validate_depth()
        .map_err(|message| IndexedDbError::Data(message.to_owned()))?;
    let mut output = Vec::new();
    encode_into(key, encoding, &mut output)?;
    Ok(output)
}

fn append(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), IndexedDbError> {
    output.try_reserve(bytes.len()).map_err(|error| {
        IndexedDbError::Data(format!(
            "IndexedDB key encoding exceeds available memory: {error}"
        ))
    })?;
    output.extend_from_slice(bytes);
    Ok(())
}

fn escaped(output: &mut Vec<u8>, byte: u8) -> Result<(), IndexedDbError> {
    if byte == 0 {
        append(output, &[0, 0xff])
    } else {
        append(output, &[byte])
    }
}

fn encode_into(key: &Key, encoding: Encoding, output: &mut Vec<u8>) -> Result<(), IndexedDbError> {
    match key {
        Key::Number(number) => {
            append(output, &[NUMBER])?;
            let value = number.value();
            let bits = match encoding {
                Encoding::Value => value.to_bits(),
                Encoding::Ordered => {
                    let bits = if value == 0.0 { 0 } else { value.to_bits() };
                    if bits & SIGN_BIT == 0 {
                        bits ^ SIGN_BIT
                    } else {
                        !bits
                    }
                }
            };
            append(output, &bits.to_be_bytes())?;
        }
        Key::Date(milliseconds) => {
            if !(-MAX_DATE..=MAX_DATE).contains(milliseconds) {
                return Err(IndexedDbError::Data(
                    "invalid IndexedDB Date key".to_owned(),
                ));
            }
            append(output, &[DATE])?;
            let mut bytes = milliseconds.to_be_bytes();
            bytes[0] ^= 0x80;
            append(output, &bytes)?;
        }
        Key::String(units) => {
            append(output, &[STRING])?;
            for unit in units {
                for byte in unit.to_be_bytes() {
                    escaped(output, byte)?;
                }
            }
            append(output, &[0, 0])?;
        }
        Key::Binary(bytes) => {
            append(output, &[BINARY])?;
            for byte in bytes {
                escaped(output, *byte)?;
            }
            append(output, &[0, 0])?;
        }
        Key::Array(keys) => {
            append(output, &[ARRAY])?;
            for key in keys {
                encode_into(key, encoding, output)?;
            }
            append(output, &[0])?;
        }
    }
    Ok(())
}

fn corruption(message: &str) -> IndexedDbError {
    IndexedDbError::Corruption(format!("invalid IndexedDB binary key: {message}"))
}

fn decode(bytes: &[u8], encoding: Encoding) -> Result<Key, IndexedDbError> {
    let mut input = Decoder { bytes, offset: 0 };
    let key = input.key(encoding, 0)?;
    if input.offset != bytes.len() {
        return Err(corruption("trailing bytes"));
    }
    Ok(key)
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl Decoder<'_> {
    fn byte(&mut self) -> Result<u8, IndexedDbError> {
        let byte = self
            .bytes
            .get(self.offset)
            .copied()
            .ok_or_else(|| corruption("truncated input"))?;
        self.offset += 1;
        Ok(byte)
    }

    fn scalar(&mut self) -> Result<[u8; 8], IndexedDbError> {
        let end = self
            .offset
            .checked_add(8)
            .ok_or_else(|| corruption("scalar length overflow"))?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| corruption("truncated scalar"))?;
        self.offset = end;
        Ok(bytes.try_into().expect("eight-byte scalar"))
    }

    fn variable(&mut self) -> Result<Vec<u8>, IndexedDbError> {
        let mut output = Vec::new();
        loop {
            let byte = self.byte()?;
            if byte != 0 {
                append(&mut output, &[byte])?;
                continue;
            }
            match self.byte()? {
                0 => return Ok(output),
                0xff => append(&mut output, &[0])?,
                _ => return Err(corruption("invalid variable-length escape")),
            }
        }
    }

    fn key(&mut self, encoding: Encoding, depth: usize) -> Result<Key, IndexedDbError> {
        match self.byte()? {
            NUMBER => {
                let payload = u64::from_be_bytes(self.scalar()?);
                let bits = match encoding {
                    Encoding::Value => payload,
                    Encoding::Ordered if payload & SIGN_BIT != 0 => payload ^ SIGN_BIT,
                    Encoding::Ordered => !payload,
                };
                let value = f64::from_bits(bits);
                if matches!(encoding, Encoding::Ordered) && value == 0.0 && bits != 0 {
                    return Err(corruption("noncanonical zero"));
                }
                Key::number(value).ok_or_else(|| corruption("NaN"))
            }
            DATE => {
                let mut bytes = self.scalar()?;
                bytes[0] ^= 0x80;
                let value = i64::from_be_bytes(bytes);
                if !(-MAX_DATE..=MAX_DATE).contains(&value) {
                    return Err(corruption("Date outside the supported range"));
                }
                Ok(Key::Date(value))
            }
            STRING => {
                let bytes = self.variable()?;
                if bytes.len() % 2 != 0 {
                    return Err(corruption("incomplete UTF-16 code unit"));
                }
                Ok(Key::String(
                    bytes
                        .chunks_exact(2)
                        .map(|unit| u16::from_be_bytes([unit[0], unit[1]]))
                        .collect(),
                ))
            }
            BINARY => Ok(Key::Binary(self.variable()?)),
            ARRAY => {
                if depth >= MAX_KEY_DEPTH {
                    return Err(corruption("maximum array nesting exceeded"));
                }
                let mut keys = Vec::new();
                loop {
                    match self.bytes.get(self.offset) {
                        Some(0) => {
                            self.offset += 1;
                            return Ok(Key::Array(keys));
                        }
                        Some(_) => {
                            let key = self.key(encoding, depth + 1)?;
                            keys.try_reserve(1)
                                .map_err(|_| corruption("array too large"))?;
                            keys.push(key);
                        }
                        None => return Err(corruption("unterminated array")),
                    }
                }
            }
            _ => Err(corruption("unknown type tag")),
        }
    }
}

#[cfg(test)]
mod tests;
