//! Native storage latency harness. Run the same workload before and after
//! replacing the backend; each sample starts with an independently seeded DB.
use std::{
    error::Error,
    fs,
    hint::black_box,
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use moli_indexeddb::{IndexedDbManager, Key, ObjectStoreOptions, OpenOptions, TransactionMode};

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mode = args.first().map(String::as_str).unwrap_or("memory");
    if !matches!(mode, "memory" | "disk") {
        return Err("usage: storage_bench [memory|disk] [record-count ...]".into());
    }
    let counts = if args.len() > 1 {
        args[1..]
            .iter()
            .map(|count| count.parse::<usize>())
            .collect::<Result<Vec<_>, _>>()?
    } else {
        vec![1_000, 10_000, 100_000]
    };
    if counts.contains(&0) {
        return Err("record count must be positive".into());
    }
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "moli-indexeddb-bench-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
    )));
    println!("mode,records,sample,operation,elapsed_ns");
    for count in counts {
        let count_i64 = i64::try_from(count)?;
        for sample in 0..7 {
            let mut manager = if mode == "memory" {
                IndexedDbManager::new_in_memory()
            } else {
                IndexedDbManager::new(scratch.0.join(format!("{count}-{sample}")))?
            };
            let opened = manager.open(OpenOptions {
                origin: "storage-key:v1;origin=https://indexeddb-bench.example".to_owned(),
                name: "database".to_owned(),
                version: Some(1),
            })?;
            let upgrade = opened
                .upgrade_transaction
                .ok_or("expected fresh database")?;
            manager.create_object_store(upgrade, "store", ObjectStoreOptions::default())?;
            manager.commit_transaction(upgrade)?;
            let store = ["store".to_owned()];
            let seed =
                manager.begin_transaction(opened.database, &store, TransactionMode::ReadWrite)?;
            for key in 0..count_i64 {
                manager.put(seed, "store", Some(Key::from(key)), vec![0x61; 128])?;
            }
            manager.commit_transaction(seed)?;

            for operation in ["read", "update", "delete"] {
                let start = Instant::now();
                let mode = if operation == "read" {
                    TransactionMode::ReadOnly
                } else {
                    TransactionMode::ReadWrite
                };
                let transaction = manager.begin_transaction(opened.database, &store, mode)?;
                let key = Key::from(count_i64 / 2);
                match operation {
                    "read" => {
                        black_box(manager.get(transaction, "store", &key)?);
                    }
                    "update" => {
                        manager.put(transaction, "store", Some(key), vec![0x62; 128])?;
                    }
                    _ => manager.delete(transaction, "store", &key)?,
                }
                manager.commit_transaction(transaction)?;
                let elapsed = start.elapsed().as_nanos();
                println!(
                    "{},{count},{sample},{operation},{elapsed}",
                    args.first().map(String::as_str).unwrap_or("memory")
                );
            }
        }
    }
    Ok(())
}
