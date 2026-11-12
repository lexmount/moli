// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Xtask utilities for Parley, currently only integrates Kompari

use clap::Parser;
use kompari::DirDiffConfig;
use kompari_tasks::{Actions, Args, Task};
use std::path::Path;
use std::process::Command;

struct ActionsImpl();

impl Actions for ActionsImpl {
    fn generate_all_tests(&self) -> kompari::Result<()> {
        let cargo = std::env::var("CARGO").unwrap();
        Command::new(&cargo)
            .arg("test")
            .env("PARLEY_TEST", "generate-all")
            .status()?;
        Ok(())
    }
}

fn main() -> kompari::Result<()> {
    let tests_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("parley_tests");

    let snapshots_path = tests_path.join("snapshots");
    let current_path = tests_path.join("current");

    let args = Args::parse();
    let mut diff_config = DirDiffConfig::new(snapshots_path, current_path);
    diff_config.set_ignore_right_missing(true);
    let actions = ActionsImpl();
    let mut task = Task::new(diff_config, Box::new(actions));
    task.run(&args)?;
    Ok(())
}
