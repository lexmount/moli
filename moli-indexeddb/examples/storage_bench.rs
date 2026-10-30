//! Native SQLite command latency, including worker dispatch and transaction
//! admission. Each sample uses a fresh logical database and one real connection.
use moli_indexeddb::sqlite::{
    Query,
    service::{Command, CommandResult, Completion, DatabaseClient, Service},
};
use moli_indexeddb::{Key, KeyRange, ObjectStoreOptions, TransactionHandle, TransactionMode};
use std::{
    collections::BTreeSet,
    error::Error,
    fs,
    hint::black_box,
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn receive(receiver: mpsc::Receiver<Completion>) -> Result<Completion, Box<dyn Error>> {
    let reply = receiver.recv_timeout(Duration::from_secs(60))?;
    if let Err(error) = &reply.result {
        return Err(Box::new(error.clone()));
    }
    Ok(reply)
}

fn execute(client: &DatabaseClient, command: Command) -> Result<Completion, Box<dyn Error>> {
    let (sender, receiver) = mpsc::channel();
    client.submit(command, move |reply| {
        let _ = sender.send(reply);
    })?;
    receive(receiver)
}

fn begin(
    client: &DatabaseClient,
    transaction: TransactionHandle,
    mode: TransactionMode,
    admission: Option<moli_indexeddb::TransactionRequestHandle>,
) -> Result<(), Box<dyn Error>> {
    execute(
        client,
        Command::Begin {
            transaction,
            mode,
            stores: BTreeSet::from(["store".into()]),
            version: None,
            admission,
        },
    )?;
    Ok(())
}

fn write(transaction: TransactionHandle, key: Key, byte: u8) -> Command {
    Command::Write {
        transaction,
        store: "store".into(),
        key: Some(key),
        value: vec![byte; 128].into(),
        indexes: vec![],
        add_only: false,
        quota: None,
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
        "moli-indexeddb-sqlite-bench-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    )));
    if mode == "disk" {
        fs::create_dir(&scratch.0)?;
    }
    println!("mode,records,sample,operation,elapsed_ns");
    for count in counts {
        let count_i64 = i64::try_from(count)?;
        for sample in 0..7 {
            let mut service = if mode == "memory" {
                Service::new(Default::default())?
            } else {
                Service::on_disk(
                    scratch.0.join(format!("{count}-{sample}")),
                    Default::default(),
                )?
            };
            let (sender, receiver) = mpsc::channel();
            let client = service.resolve(
                b"storage-key:v1;origin=https://indexeddb-bench.example".to_vec(),
                "database".into(),
                move |reply| {
                    let _ = sender.send(reply);
                },
            )?;
            receive(receiver)?;
            let connection = client.open_connection(Arc::new(|| {}))?;
            let transaction = client.allocate_transaction_handle()?;
            let (sender, receiver) = mpsc::channel();
            connection.begin_upgrade(transaction, 1, move |reply| {
                let _ = sender.send(reply);
            })?;
            receive(receiver)?;
            execute(
                &client,
                Command::CreateStore {
                    transaction,
                    name: "store".into(),
                    options: ObjectStoreOptions::default(),
                },
            )?;
            execute(
                &client,
                Command::Commit {
                    transaction,
                    quota: None,
                },
            )?;
            let stores = [moli_indexeddb::IndexedDbName::from("store")];
            let transaction = client.allocate_transaction_handle()?;
            let lease = connection.queue_transaction_start(
                &stores,
                TransactionMode::ReadWrite,
                Arc::new(|| {}),
            )?;
            begin(
                &client,
                transaction,
                TransactionMode::ReadWrite,
                Some(lease.handle().clone()),
            )?;
            for key in 0..count_i64 {
                execute(&client, write(transaction, key.into(), 0x61))?;
            }
            execute(
                &client,
                Command::Commit {
                    transaction,
                    quota: None,
                },
            )?;
            drop(lease);
            for operation in ["read", "update", "delete"] {
                let started = Instant::now();
                let tx_mode = if operation == "read" {
                    TransactionMode::ReadOnly
                } else {
                    TransactionMode::ReadWrite
                };
                let transaction = client.allocate_transaction_handle()?;
                let lease =
                    connection.queue_transaction_start(&stores, tx_mode, Arc::new(|| {}))?;
                begin(&client, transaction, tx_mode, Some(lease.handle().clone()))?;
                let key = Key::from(count_i64 / 2);
                let range = KeyRange {
                    lower: Some(key.clone()),
                    upper: Some(key.clone()),
                    lower_open: false,
                    upper_open: false,
                };
                match operation {
                    "read" => {
                        let reply = execute(
                            &client,
                            Command::Query {
                                transaction,
                                store: "store".into(),
                                query: Query {
                                    range,
                                    ..Default::default()
                                },
                            },
                        )?;
                        match &reply.result {
                            Ok(CommandResult::Records(rows))
                                if rows.len() == 1
                                    && rows[0].primary_key == key
                                    && rows[0]
                                        .value
                                        .as_ref()
                                        .is_some_and(|value| value.wire_bytes() == [0x61; 128]) => {
                            }
                            _ => return Err("benchmark read returned unexpected data".into()),
                        }
                        black_box(reply);
                    }
                    "update" => {
                        execute(&client, write(transaction, key, 0x62))?;
                    }
                    _ => {
                        execute(
                            &client,
                            Command::Delete {
                                transaction,
                                store: "store".into(),
                                range,
                            },
                        )?;
                    }
                }
                execute(
                    &client,
                    Command::Commit {
                        transaction,
                        quota: None,
                    },
                )?;
                drop(lease);
                println!(
                    "{mode},{count},{sample},{operation},{}",
                    started.elapsed().as_nanos()
                );
            }
            service.shutdown()?;
        }
    }
    Ok(())
}
