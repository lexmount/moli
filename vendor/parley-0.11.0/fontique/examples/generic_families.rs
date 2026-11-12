// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Print an enumeration of discovered fonts for each `GenericFamily`.

use fontique::{Collection, CollectionOptions, GenericFamily::*};

fn main() {
    let mut collection = Collection::new(CollectionOptions::default());

    for gf in [
        Serif,
        SansSerif,
        Monospace,
        Cursive,
        Fantasy,
        SystemUi,
        UiSerif,
        UiSansSerif,
        UiMonospace,
        UiRounded,
        Emoji,
        Math,
        FangSong,
    ] {
        println!("GenericFamily::{gf:?}:");

        let ids = collection.generic_families(gf).collect::<Vec<_>>();
        for id in ids {
            if let Some(name) = collection.family_name(id) {
                println!("{name}");
            }
        }

        println!();
    }
}
