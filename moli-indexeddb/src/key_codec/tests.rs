use super::*;
use rusqlite::{Connection, params};

fn number(value: f64) -> Key {
    Key::number(value).unwrap()
}

fn edge_keys() -> Vec<Key> {
    vec![
        number(f64::NEG_INFINITY),
        number(-f64::MAX),
        number(-1.5),
        number(-f64::MIN_POSITIVE),
        number(-f64::from_bits(1)),
        number(-0.0),
        number(0.0),
        number(f64::from_bits(1)),
        number(f64::MIN_POSITIVE),
        number(1.5),
        number(9_007_199_254_740_992.0),
        number(f64::MAX),
        number(f64::INFINITY),
        Key::Date(-MAX_DATE),
        Key::Date(-1),
        Key::Date(0),
        Key::Date(MAX_DATE),
        Key::String(vec![]),
        Key::String(vec![0]),
        Key::String(vec![0, 1]),
        Key::String(vec![1]),
        Key::String(vec![0xd800]),
        Key::String(vec![0xd800, 0xdc00]),
        Key::String(vec![0xd801]),
        Key::String(vec![0xe000]),
        Key::String(vec![0xffff]),
        Key::Binary(vec![]),
        Key::Binary(vec![0]),
        Key::Binary(vec![0, 0]),
        Key::Binary(vec![0, 1]),
        Key::Binary(vec![1]),
        Key::Binary(vec![0xff]),
        Key::Array(vec![]),
        Key::Array(vec![number(0.0)]),
        Key::Array(vec![number(0.0), number(0.0)]),
        Key::Array(vec![Key::String(vec![])]),
        Key::Array(vec![Key::Array(vec![])]),
    ]
}

// Reproducible samples without introducing a random-number dependency.
struct Samples(u64);

impl Samples {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut bits = self.0;
        bits = (bits ^ (bits >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        bits = (bits ^ (bits >> 27)).wrapping_mul(0x94d049bb133111eb);
        bits ^ (bits >> 31)
    }

    fn key(&mut self, depth: usize) -> Key {
        match self.next() % if depth < 5 { 5 } else { 4 } {
            0 => Key::number(f64::from_bits(self.next())).unwrap_or_else(|| number(0.0)),
            1 => {
                let value = i64::try_from(self.next() % (2 * MAX_DATE as u64 + 1)).unwrap();
                Key::Date(value - MAX_DATE)
            }
            2 => {
                let count = self.next() % 12;
                Key::String((0..count).map(|_| self.next() as u16).collect())
            }
            3 => {
                let count = self.next() % 12;
                Key::Binary((0..count).map(|_| self.next() as u8).collect())
            }
            _ => {
                let count = self.next() % 4;
                Key::Array((0..count).map(|_| self.key(depth + 1)).collect())
            }
        }
    }
}

#[test]
fn ordered_keys_match_native_comparison_and_sqlite_blob_order() {
    let mut samples = Samples(0x6d6f6c692d696462);
    let mut keys = edge_keys();
    keys.extend((0..10_000).map(|_| samples.key(0)));
    let encoded = keys
        .iter()
        .map(|key| key.encode_ordered().unwrap())
        .collect::<Vec<_>>();
    for (key, bytes) in keys.iter().zip(&encoded) {
        assert_eq!(Key::decode_ordered(bytes).unwrap(), *key);
        let value = key.encode_value().unwrap();
        assert_eq!(
            Key::decode_value(&value).unwrap().encode_value().unwrap(),
            value
        );
    }
    for _ in 0..100_000 {
        let left = samples.next() as usize % keys.len();
        let right = samples.next() as usize % keys.len();
        assert_eq!(
            keys[left].cmp(&keys[right]),
            encoded[left].cmp(&encoded[right])
        );
    }

    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE keys (key_sort BLOB PRIMARY KEY, key_value BLOB NOT NULL)
             STRICT, WITHOUT ROWID",
        )
        .unwrap();
    let transaction = connection.transaction().unwrap();
    for (key, bytes) in keys.iter().zip(&encoded) {
        transaction
            .execute(
                "INSERT INTO keys VALUES (?1, ?2) ON CONFLICT(key_sort) DO NOTHING",
                params![bytes, key.encode_value().unwrap()],
            )
            .unwrap();
    }
    transaction.commit().unwrap();
    keys.sort();
    keys.dedup();
    let stored = connection
        .prepare("SELECT key_value FROM keys ORDER BY key_sort")
        .unwrap()
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .unwrap()
        .map(|row| Key::decode_value(&row.unwrap()).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(stored, keys);
}

#[test]
fn zeros_share_ordering_identity_but_preserve_value_bits() {
    let negative = number(-0.0);
    let positive = number(0.0);
    assert_eq!(
        negative.encode_ordered().unwrap(),
        positive.encode_ordered().unwrap()
    );
    assert_ne!(
        negative.encode_value().unwrap(),
        positive.encode_value().unwrap()
    );
    for original in [negative, positive] {
        let restored = Key::decode_value(&original.encode_value().unwrap()).unwrap();
        let (Key::Number(original), Key::Number(restored)) = (original, restored) else {
            panic!("number roundtrip");
        };
        assert_eq!(original.value().to_bits(), restored.value().to_bits());
    }
}

#[test]
fn binary_key_golden_vectors_preserve_type_and_prefix_order() {
    assert_eq!(
        number(0.0).encode_ordered().unwrap(),
        [0x10, 0x80, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        Key::Date(0).encode_ordered().unwrap(),
        [0x20, 0x80, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(
        Key::String(vec![0]).encode_ordered().unwrap(),
        [0x30, 0, 0xff, 0, 0xff, 0, 0]
    );
    assert_eq!(
        Key::Binary(vec![0]).encode_ordered().unwrap(),
        [0x40, 0, 0xff, 0, 0]
    );
    assert_eq!(Key::Array(vec![]).encode_ordered().unwrap(), [0x50, 0]);
}

#[test]
fn corrupt_and_truncated_binary_keys_are_rejected() {
    for key in [
        vec![],
        vec![0],
        vec![0x11],
        vec![STRING, 0],
        vec![STRING, 0, 1],
        vec![STRING, 1, 0, 0],
        vec![BINARY, 0, 0xfe],
        vec![ARRAY],
        vec![ARRAY, 0x11, 0],
        vec![ARRAY, ARRAY, 0],
    ] {
        assert!(Key::decode_ordered(&key).is_err(), "{key:?}");
        assert!(Key::decode_value(&key).is_err(), "{key:?}");
    }
    for key in edge_keys() {
        let bytes = key.encode_ordered().unwrap();
        for length in 0..bytes.len() {
            assert!(Key::decode_ordered(&bytes[..length]).is_err());
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert!(Key::decode_ordered(&trailing).is_err());
    }
    for value in [f64::NAN, f64::from_bits(0xfff0000000000001)] {
        let bits = value.to_bits();
        let ordered = if bits & SIGN_BIT == 0 {
            bits ^ SIGN_BIT
        } else {
            !bits
        };
        let mut bytes = vec![NUMBER];
        bytes.extend_from_slice(&ordered.to_be_bytes());
        assert!(Key::decode_ordered(&bytes).is_err());
        bytes[1..].copy_from_slice(&bits.to_be_bytes());
        assert!(Key::decode_value(&bytes).is_err());
    }
    let mut negative_zero = vec![NUMBER];
    negative_zero.extend_from_slice(&(!(-0.0f64).to_bits()).to_be_bytes());
    assert!(Key::decode_ordered(&negative_zero).is_err());
    for invalid in [i64::MIN, -MAX_DATE - 1, MAX_DATE + 1, i64::MAX] {
        assert!(Key::Date(invalid).encode_ordered().is_err());
        let mut bytes = vec![DATE];
        let mut payload = invalid.to_be_bytes();
        payload[0] ^= 0x80;
        bytes.extend_from_slice(&payload);
        assert!(Key::decode_ordered(&bytes).is_err());
    }
}

#[test]
fn binary_array_depth_matches_the_native_limit() {
    let mut key = number(1.0);
    for _ in 0..MAX_KEY_DEPTH {
        key = Key::Array(vec![key]);
    }
    let encoded = key.encode_ordered().unwrap();
    assert_eq!(Key::decode_ordered(&encoded).unwrap(), key);
    assert!(Key::Array(vec![key]).encode_ordered().is_err());
    let mut too_deep = vec![ARRAY];
    too_deep.extend_from_slice(&encoded);
    too_deep.push(0);
    assert!(Key::decode_ordered(&too_deep).is_err());
}
