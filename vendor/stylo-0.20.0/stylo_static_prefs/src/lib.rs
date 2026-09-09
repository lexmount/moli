/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::sync::atomic::{AtomicBool, AtomicI32};

include!(concat!(env!("OUT_DIR"), "/generated.rs"));

#[test]
fn test_basic_preferences() {
    assert!(!pref!("layout.unimplemented"));
    set_pref!("layout.unimplemented", true);
    assert!(pref!("layout.unimplemented"));

    assert_eq!(pref!("layout.threads"), -1);
    set_pref!("layout.threads", 42);
    assert_eq!(pref!("layout.threads"), 42);
}
