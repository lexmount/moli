use super::{BoundedByteBuffer, ByteLimits, InsertOutcome};

#[test]
fn accepts_entries_at_exact_limits() {
    let mut buffer = BoundedByteBuffer::new(ByteLimits::new(4, 4));

    assert_eq!(
        buffer.insert("one", "body", 4),
        InsertOutcome::Stored {
            evicted: Vec::new()
        }
    );
    assert_eq!(buffer.used_bytes(), 4);
    assert_eq!(buffer.get(&"one"), Some(&"body"));
}

#[test]
fn rejects_an_entry_over_either_limit_without_evicting_other_entries() {
    let mut buffer = BoundedByteBuffer::new(ByteLimits::new(6, 4));
    assert!(matches!(
        buffer.insert("kept", "1234", 4),
        InsertOutcome::Stored { .. }
    ));

    assert_eq!(
        buffer.insert("too-large", "12345", 5),
        InsertOutcome::Rejected {
            key: "too-large",
            value: "12345"
        }
    );
    assert_eq!(buffer.used_bytes(), 4);
    assert_eq!(buffer.get(&"kept"), Some(&"1234"));
    assert!(!buffer.contains_key(&"too-large"));

    let mut total_limited = BoundedByteBuffer::new(ByteLimits::new(4, 8));
    assert_eq!(
        total_limited.insert("over-total", "12345", 5),
        InsertOutcome::Rejected {
            key: "over-total",
            value: "12345"
        }
    );
    assert!(total_limited.is_empty());
}

#[test]
fn evicts_oldest_entries_until_the_total_budget_fits() {
    let mut buffer = BoundedByteBuffer::new(ByteLimits::new(5, 4));
    assert!(matches!(
        buffer.insert("first", "aa", 2),
        InsertOutcome::Stored { .. }
    ));
    assert!(matches!(
        buffer.insert("second", "bb", 2),
        InsertOutcome::Stored { .. }
    ));

    assert_eq!(
        buffer.insert("third", "ccc", 3),
        InsertOutcome::Stored {
            evicted: vec![("first", "aa")]
        }
    );
    assert_eq!(buffer.used_bytes(), 5);
    assert_eq!(buffer.get(&"second"), Some(&"bb"));
    assert_eq!(buffer.get(&"third"), Some(&"ccc"));
}

#[test]
fn replacing_an_entry_releases_its_charge_and_makes_it_newest() {
    let mut buffer = BoundedByteBuffer::new(ByteLimits::new(5, 4));
    let _ = buffer.insert("first", "aa", 2);
    let _ = buffer.insert("second", "bb", 2);
    let _ = buffer.insert("first", "a", 1);

    assert_eq!(
        buffer.insert("third", "ccc", 3),
        InsertOutcome::Stored {
            evicted: vec![("second", "bb")]
        }
    );
    assert_eq!(buffer.used_bytes(), 4);
    assert_eq!(buffer.get(&"first"), Some(&"a"));
    assert_eq!(buffer.get(&"third"), Some(&"ccc"));
}

#[test]
fn rejected_replacement_removes_the_previous_value() {
    let mut buffer = BoundedByteBuffer::new(ByteLimits::new(4, 4));
    let _ = buffer.insert("entry", "old", 3);

    assert_eq!(
        buffer.insert("entry", "oversized", 5),
        InsertOutcome::Rejected {
            key: "entry",
            value: "oversized"
        }
    );
    assert!(buffer.is_empty());
    assert_eq!(buffer.used_bytes(), 0);
}

#[test]
fn remove_and_clear_return_all_byte_charges() {
    let mut buffer = BoundedByteBuffer::new(ByteLimits::new(8, 4));
    let _ = buffer.insert("first".to_owned(), "aa", 2);
    let _ = buffer.insert("second".to_owned(), "bbb", 3);

    assert_eq!(buffer.remove("first"), Some("aa"));
    assert_eq!(buffer.used_bytes(), 3);
    assert_eq!(buffer.len(), 1);

    buffer.clear();
    assert!(buffer.is_empty());
    assert_eq!(buffer.used_bytes(), 0);
}
