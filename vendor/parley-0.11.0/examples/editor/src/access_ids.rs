// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use accesskit::NodeId;
use core::sync::atomic::{AtomicU64, Ordering};

pub const WINDOW_ID: NodeId = NodeId(0);
pub const TEXT_INPUT_ID: NodeId = NodeId(1);

pub fn next_node_id() -> NodeId {
    static NEXT: AtomicU64 = AtomicU64::new(2);
    NodeId(NEXT.fetch_add(1, Ordering::Relaxed))
}
