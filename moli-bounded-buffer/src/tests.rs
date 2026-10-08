use super::{BoundedByteBuffer, ByteLimits, InsertOutcome};

#[test]
fn shrinking_entry_limit_keeps_interleaved_survivors_in_fifo_order() {
    let mut buffer = BoundedByteBuffer::new(ByteLimits::new(100, 20));
    for (key, bytes) in [(0, 10), (1, 1), (2, 10), (3, 1), (4, 10)] {
        buffer.insert(key, key, bytes);
    }
    assert_eq!(
        buffer.set_limits(ByteLimits::new(100, 1)),
        vec![(0, 0), (2, 2), (4, 4)]
    );
    assert_eq!(buffer.used_bytes(), 2);
    assert_eq!(buffer.set_limits(ByteLimits::new(1, 1)), vec![(1, 1)]);
    assert_eq!(buffer.get(&3), Some(&3));
    assert_eq!(buffer.len(), 1);
}

#[test]
fn changing_limits_evicts_oversized_then_oldest_without_reordering_survivors() {
    let mut buffer = BoundedByteBuffer::new(ByteLimits::new(20, 10));
    buffer.insert("a", 1, 3);
    buffer.insert("b", 2, 6);
    buffer.insert("c", 3, 3);
    assert_eq!(
        buffer.set_limits(ByteLimits::new(4, 4)),
        vec![("b", 2), ("a", 1)]
    );
    assert_eq!(buffer.used_bytes(), 3);
    assert_eq!(buffer.get(&"c"), Some(&3));
    assert!(buffer.set_limits(ByteLimits::new(10, 10)).is_empty());
    assert_eq!(
        buffer.insert("d", 4, 8),
        InsertOutcome::Stored {
            evicted: vec![("c", 3)]
        }
    );
    assert_eq!(buffer.set_limits(ByteLimits::new(0, 0)), vec![("d", 4)]);
    assert!(buffer.is_empty());
}

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

#[test]
fn retain_visits_once_preserves_fifo_and_releases_only_removed_charges() {
    let mut buffer = BoundedByteBuffer::new(ByteLimits::new(10, 10));
    for key in 0..4 {
        let _ = buffer.insert(key, key * 10, 2);
    }
    let mut visited = Vec::new();
    let removed = buffer.retain(|key, value| {
        visited.push(*key);
        *value += 1;
        key % 2 == 0
    });
    assert_eq!(visited, vec![0, 1, 2, 3]);
    assert_eq!(removed, vec![(1, 11), (3, 31)]);
    assert_eq!(buffer.used_bytes(), 4);
    assert_eq!(buffer.get(&2), Some(&21));
    assert_eq!(
        buffer.insert(4, 40, 8),
        InsertOutcome::Stored {
            evicted: vec![(0, 1)]
        }
    );
    assert!(buffer.retain(|_, _| true).is_empty());
    assert_eq!(buffer.used_bytes(), 10);
    assert_eq!(buffer.retain(|_, _| false), vec![(2, 21), (4, 40)]);
    assert_eq!(buffer.used_bytes(), 0);
    assert!(buffer.is_empty());
    assert!(
        buffer
            .retain(|_, _| panic!("empty buffer must not visit"))
            .is_empty()
    );
}
