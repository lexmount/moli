// Huffman encoding is a compression technique that replaces common strings with shorter codes.
// Ugh I wish we didn't have to implement this, but the other endpoint is allowed to use it.

// Taken from https://github.com/hyperium/h3/blob/master/h3/src/qpack/prefix_string/decode.rs
// License: MIT

#[derive(Debug, Default, PartialEq, Clone)]
pub struct BitWindow {
    pub byte: u32,
    pub bit: u32,
    pub count: u32,
}

impl BitWindow {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn forwards(&mut self, step: u32) {
        self.bit += self.count;

        self.byte += self.bit / 8;
        self.bit %= 8;

        self.count = step;
    }

    pub fn opposite_bit_window(&self) -> BitWindow {
        BitWindow {
            byte: self.byte,
            bit: self.bit,
            count: 8 - (self.bit % 8),
        }
    }
}

use thiserror::Error;

#[derive(Error, Debug, PartialEq, Clone)]
pub enum Error {
    #[error("missing bits: {0:?}")]
    MissingBits(BitWindow),

    #[error("unhandled: {0:?} {1:?}")]
    Unhandled(BitWindow, usize),
}

#[derive(Clone, Debug)]
enum DecodeValue {
    Partial(&'static HuffmanDecoder),
    Sym(u8),
}

#[derive(Clone, Debug)]
struct HuffmanDecoder {
    lookup: u32,
    table: &'static [DecodeValue],
}

impl HuffmanDecoder {
    fn check_eof(&self, bit_pos: &mut BitWindow, input: &[u8]) -> Result<Option<u32>, Error> {
        use std::cmp::Ordering;
        match ((bit_pos.byte + 1) as usize).cmp(&input.len()) {
            // Position is out-of-range
            Ordering::Greater => {
                return Ok(None);
            }
            // Position is on the last byte
            Ordering::Equal => {
                let side = bit_pos.opposite_bit_window();

                let rest = match read_bits(input, side.byte, side.bit, side.count) {
                    Ok(x) => x,
                    Err(()) => {
                        return Err(Error::MissingBits(side));
                    }
                };

                let eof_filler = ((2u16 << (side.count - 1)) - 1) as u8;
                if rest & eof_filler == eof_filler {
                    return Ok(None);
                }
            }
            Ordering::Less => {}
        }
        Err(Error::MissingBits(bit_pos.clone()))
    }

    fn fetch_value(&self, bit_pos: &mut BitWindow, input: &[u8]) -> Result<Option<u32>, Error> {
        match read_bits(input, bit_pos.byte, bit_pos.bit, bit_pos.count) {
            Ok(value) => Ok(Some(value as u32)),
            Err(()) => self.check_eof(bit_pos, input),
        }
    }

    fn decode_next(&self, bit_pos: &mut BitWindow, input: &[u8]) -> Result<Option<u8>, Error> {
        bit_pos.forwards(self.lookup);

        let value = match self.fetch_value(bit_pos, input) {
            Ok(Some(value)) => value as usize,
            Ok(None) => return Ok(None),
            Err(err) => return Err(err),
        };

        let at_value = match (self.table).get(value) {
            Some(x) => x,
            None => return Err(Error::Unhandled(bit_pos.clone(), value)),
        };

        match at_value {
            DecodeValue::Sym(x) => Ok(Some(*x)),
            DecodeValue::Partial(d) => d.decode_next(bit_pos, input),
        }
    }
}

/// Read `len` bits from the `src` slice at the specified position
///
/// Never read more than 8 bits at a time. `bit_offset` may be larger than 8.
fn read_bits(src: &[u8], mut byte_offset: u32, mut bit_offset: u32, len: u32) -> Result<u8, ()> {
    if len == 0 || len > 8 || src.len() as u32 * 8 < (byte_offset * 8) + bit_offset + len {
        return Err(());
    }

    // Deal with `bit_offset` > 8
    byte_offset += bit_offset / 8;
    bit_offset -= (bit_offset / 8) * 8;

    Ok(if bit_offset + len <= 8 {
        // Read all the bits from a single byte
        (src[byte_offset as usize] << bit_offset) >> (8 - len)
    } else {
        // The range of bits spans over 2 bytes
        let mut result = (src[byte_offset as usize] as u16) << 8;
        result |= src[byte_offset as usize + 1] as u16;
        ((result << bit_offset) >> (16 - len)) as u8
    })
}

macro_rules! bits_decode {
    // general way
    (
        lookup: $count:expr, [
        $($sym:expr,)*
        $(=> $sub:ident,)* ]
    ) => {
        HuffmanDecoder {
            lookup: $count,
            table: &[
                $( DecodeValue::Sym($sym as u8), )*
                $( DecodeValue::Partial(&$sub), )*
            ]
        }
    };
    // 2-final
    ( $first:expr, $second:expr ) => {
        HuffmanDecoder {
            lookup: 1,
            table: &[
                DecodeValue::Sym($first as u8),
                DecodeValue::Sym($second as u8),
            ]
        }
    };
    // 4-final
    ( $first:expr, $second:expr, $third:expr, $fourth:expr ) => {
        HuffmanDecoder {
            lookup: 2,
            table: &[
                DecodeValue::Sym($first as u8),
                DecodeValue::Sym($second as u8),
                DecodeValue::Sym($third as u8),
                DecodeValue::Sym($fourth as u8),
            ]
        }
    };
    // 2-final-partial
    ( $first:expr, => $second:ident ) => {
        HuffmanDecoder {
            lookup: 1,
            table: &[
                DecodeValue::Sym($first as u8),
                DecodeValue::Partial(&$second),
            ]
        }
    };
    // 2-partial
    ( => $first:ident, => $second:ident ) => {
        HuffmanDecoder {
            lookup: 1,
            table: &[
                DecodeValue::Partial(&$first),
                DecodeValue::Partial(&$second),
            ]
        }
    };
    // 4-partial
    ( => $first:ident, => $second:ident,
      => $third:ident, => $fourth:ident ) => {
        HuffmanDecoder {
            lookup: 2,
            table: &[
                DecodeValue::Partial(&$first),
                DecodeValue::Partial(&$second),
                DecodeValue::Partial(&$third),
                DecodeValue::Partial(&$fourth),
            ]
        }
    };
    [ $( $name:ident => ( $($value:tt)* ), )* ] => {
        $( const $name: HuffmanDecoder = bits_decode!( $( $value )* ); )*
    };
}

#[rustfmt::skip]
bits_decode![
    HPACK_STRING => (
        lookup: 5, [ b'0', b'1', b'2', b'a', b'c', b'e', b'i', b'o', b's', b't',
        => END0_01010, => END0_01011, => END0_01100, => END0_01101,
        => END0_01110, => END0_01111, => END0_10000, => END0_10001,
        => END0_10010, => END0_10011, => END0_10100, => END0_10101,
        => END0_10110, => END0_10111, => END0_11000, => END0_11001,
        => END0_11010, => END0_11011, => END0_11100, => END0_11101,
        => END0_11110, => END0_11111,
        ]),
    END0_01010 => ( 32, b'%'),
    END0_01011 => (b'-', b'.'),
    END0_01100 => (b'/', b'3'),
    END0_01101 => (b'4', b'5'),
    END0_01110 => (b'6', b'7'),
    END0_01111 => (b'8', b'9'),
    END0_10000 => (b'=', b'A'),
    END0_10001 => (b'_', b'b'),
    END0_10010 => (b'd', b'f'),
    END0_10011 => (b'g', b'h'),
    END0_10100 => (b'l', b'm'),
    END0_10101 => (b'n', b'p'),
    END0_10110 => (b'r', b'u'),
    END0_10111 => (b':', b'B', b'C', b'D'),
    END0_11000 => (b'E', b'F', b'G', b'H'),
    END0_11001 => (b'I', b'J', b'K', b'L'),
    END0_11010 => (b'M', b'N', b'O', b'P'),
    END0_11011 => (b'Q', b'R', b'S', b'T'),
    END0_11100 => (b'U', b'V', b'W', b'Y'),
    END0_11101 => (b'j', b'k', b'q', b'v'),
    END0_11110 => (b'w', b'x', b'y', b'z'),
    END0_11111 => (=> END5_00, => END5_01, => END5_10, => END5_11),
    END5_00 => (b'&', b'*'),
    END5_01 => (b',', 59),
    END5_10 => (b'X', b'Z'),
    END5_11 => (=> END7_0, => END7_1),
    END7_0 => (b'!', b'"', b'(', b')'),
    END7_1 => (=> END8_0, => END8_1),
    END8_0 => (b'?', => END9A_1),
    END9A_1 => (b'\'', b'+'),
    END8_1 => (lookup: 2, [b'|', => END9B_01, => END9B_10, => END9B_11,]),
    END9B_01 => (b'#', b'>'),
    END9B_10 => (0, b'$', b'@', b'['),
    END9B_11 => (lookup: 2, [b']', b'~', => END13_10, => END13_11,]),
    END13_10 => (b'^', b'}'),
    END13_11 => (=> END14_0, => END14_1),
    END14_0 => (b'<', b'`'),
    END14_1 => (b'{', => END15_1),
    END15_1 =>
    (lookup: 4, [ b'\\', 195, 208, => END19_0011,
     => END19_0100, => END19_0101, => END19_0110, => END19_0111,
     => END19_1000, => END19_1001, => END19_1010, => END19_1011,
     => END19_1100, => END19_1101, => END19_1110, => END19_1111,
    ]),
    END19_0011 => (128, 130),
    END19_0100 => (131, 162),
    END19_0101 => (184, 194),
    END19_0110 => (224, 226),
    END19_0111 => (153, 161, 167, 172),
    END19_1000 => (176, 177, 179, 209),
    END19_1001 => (216, 217, 227, 229),
    END19_1010 => (lookup: 2, [230, => END19_1010_01, => END19_1010_10,
                   => END19_1010_11,]),
    END19_1010_01 => (129, 132),
    END19_1010_10 => (133, 134),
    END19_1010_11 => (136, 146),
    END19_1011 => (lookup: 3, [154, 156, 160, 163, 164, 169, 170, 173,]),
    END19_1100 => (lookup: 3, [178, 181, 185, 186, 187, 189, 190, 196,]),
    END19_1101 => (lookup: 3, [198, 228, 232, 233,
                   => END23A_100, => END23A_101,
                   => END23A_110, => END23A_111,]),
    END23A_100 => (  1, 135),
    END23A_101 => (137, 138),
    END23A_110 => (139, 140),
    END23A_111 => (141, 143),
    END19_1110 => (lookup: 4, [147, 149, 150, 151, 152, 155, 157, 158,
                   165, 166, 168, 174, 175, 180, 182, 183,]),
    END19_1111 => (lookup: 4, [188, 191, 197, 231, 239,
                   => END23B_0101, => END23B_0110, => END23B_0111,
                   => END23B_1000, => END23B_1001, => END23B_1010,
                   => END23B_1011, => END23B_1100, => END23B_1101,
                   => END23B_1110, => END23B_1111,]),
    END23B_0101 => (  9, 142),
    END23B_0110 => (144, 145),
    END23B_0111 => (148, 159),
    END23B_1000 => (171, 206),
    END23B_1001 => (215, 225),
    END23B_1010 => (236, 237),
    END23B_1011 => (199, 207, 234, 235),
    END23B_1100 => (lookup: 3, [192, 193, 200, 201, 202, 205, 210, 213,]),
    END23B_1101 => (lookup: 3, [218, 219, 238, 240, 242, 243, 255,
                    => END27A_111,]),
    END27A_111 => (203, 204),
    END23B_1110 => (lookup: 4, [211, 212, 214, 221, 222, 223, 241, 244,
                    245, 246, 247, 248, 250, 251, 252, 253,]),
    END23B_1111 => (lookup: 4, [ 254, => END27B_0001, => END27B_0010,
                    => END27B_0011, => END27B_0100, => END27B_0101,
                    => END27B_0110, => END27B_0111, => END27B_1000,
                    => END27B_1001, => END27B_1010, => END27B_1011,
                    => END27B_1100, => END27B_1101, => END27B_1110,
                    => END27B_1111,]),
    END27B_0001 => (2, 3),
    END27B_0010 => (4, 5),
    END27B_0011 => (6, 7),
    END27B_0100 => (8, 11),
    END27B_0101 => (12, 14),
    END27B_0110 => (15, 16),
    END27B_0111 => (17, 18),
    END27B_1000 => (19, 20),
    END27B_1001 => (21, 23),
    END27B_1010 => (24, 25),
    END27B_1011 => (26, 27),
    END27B_1100 => (28, 29),
    END27B_1101 => (30, 31),
    END27B_1110 => (127, 220),
    END27B_1111 => (lookup: 1, [249, => END31_1,]),
    END31_1 => (lookup: 2, [10, 13, 22, => EOF,]),
    EOF => (lookup: 8, []),
    ];

pub struct DecodeIter<'a> {
    bit_pos: BitWindow,
    content: &'a Vec<u8>,
}

impl Iterator for DecodeIter<'_> {
    type Item = Result<u8, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        match HPACK_STRING.decode_next(&mut self.bit_pos, self.content) {
            Ok(Some(x)) => Some(Ok(x)),
            Err(err) => Some(Err(err)),
            Ok(None) => None,
        }
    }
}

pub trait HpackStringDecode {
    fn hpack_decode(&self) -> DecodeIter<'_>;
}

impl HpackStringDecode for Vec<u8> {
    fn hpack_decode(&self) -> DecodeIter<'_> {
        DecodeIter {
            bit_pos: BitWindow::new(),
            content: self,
        }
    }
}
