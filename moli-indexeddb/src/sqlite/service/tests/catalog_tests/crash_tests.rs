//! Kill a real owner process without running SQLite/Service destructors. This
//! checks process-crash recovery, not the stronger guarantee of power-loss sync.
use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Command as ProcessCommand, Stdio},
};

const ROOT_ENV: &str = "MOLI_TEST_IDB_CRASH_ROOT";
const CASE_ENV: &str = "MOLI_TEST_IDB_CRASH_CASE";
const READY: &str = "MOLI_IDB_CRASH_READY ";
const NAME: &str = "crash-recovery";

fn preparation_boundary(case: &str) -> Option<Boundary> {
    match case {
        "marker" => Some(Boundary::MarkerSynced),
        "allocated" => Some(Boundary::Allocated),
        "creating-synced" => Some(Boundary::CreatingSynced),
        _ => None,
    }
}

fn crash_child(root: PathBuf, case: &str) {
    let service = Service::on_disk(root, config()).unwrap();
    if let Some(boundary) = preparation_boundary(case) {
        service.catalog.as_ref().unwrap().interrupt_at(boundary);
    }
    let (sender, receiver) = mpsc::channel();
    let client = service
        .resolve(OWNER.to_vec(), NAME.into(), move |reply| {
            let _ = sender.send(reply);
        })
        .unwrap();
    let completion = receive(&receiver);
    assert_eq!(
        completion.result.is_err(),
        preparation_boundary(case).is_some()
    );
    let mut active = None;
    if preparation_boundary(case).is_none() && case != "unpublished" {
        seed(&client);
        match case {
            "committed" => {}
            "write" | "prepared-write" => {
                active = Some(queue(&client, "a", TransactionMode::ReadWrite));
                start(
                    &client,
                    "a",
                    TransactionMode::ReadWrite,
                    active.as_ref().unwrap(),
                );
                if case == "write" {
                    write(&client, &[9; 8192]);
                } else {
                    assert!(matches!(
                        execute(
                            &client,
                            Command::PrepareWrite {
                                transaction: TX,
                                store: "a".into(),
                                add_only: false,
                            },
                        )
                        .unwrap(),
                        CommandResult::Prepared(_)
                    ));
                }
            }
            "upgrade" => {
                execute(
                    &client,
                    Command::Begin {
                        transaction: TX,
                        mode: TransactionMode::VersionChange,
                        stores: BTreeSet::new(),
                        version: Some(2),
                        admission: None,
                    },
                )
                .unwrap();
                execute(
                    &client,
                    Command::CreateStore {
                        transaction: TX,
                        name: "uncommitted".into(),
                        options: ObjectStoreOptions::default(),
                    },
                )
                .unwrap();
            }
            "deleted" | "renamed" | "source-synced" => {
                let boundary = match case {
                    "renamed" => Some(Boundary::Renamed),
                    "source-synced" => Some(Boundary::SourceSynced),
                    _ => None,
                };
                if let Some(boundary) = boundary {
                    service.catalog.as_ref().unwrap().interrupt_at(boundary);
                }
                assert_eq!(
                    execute(&client, Command::DeleteDatabase).is_err(),
                    boundary.is_some()
                );
            }
            _ => panic!("unknown crash case: {case}"),
        }
        if matches!(case, "committed" | "write" | "prepared-write" | "upgrade") {
            assert!(
                fs::metadata(path(&client).with_extension("sqlite-wal"))
                    .unwrap()
                    .len()
                    > 0
            );
        }
    }
    let generation = moli_crypto::sha256_hex(client.identity().generation);
    let usage = client.committed().map_or(0, |metadata| metadata.usage);
    let mut output = std::io::stdout().lock();
    writeln!(output, "{READY}{generation} {usage}").unwrap();
    output.flush().unwrap();
    // The parent kills us only after the exact SQL/catalog boundary above.
    // Keep every owner, active lease and connection alive while waiting.
    std::io::stdin().read_exact(&mut [0]).unwrap();
    drop(active);
    drop(client);
    drop(service);
    panic!("crash child must be killed, never resumed");
}

fn kill_at_boundary(root: &std::path::Path, case: &str) -> (String, u64) {
    let module = module_path!().split_once("::").unwrap().1;
    let test = format!("{module}::catalog_recovers_after_forced_process_exit");
    let mut child = ProcessCommand::new(std::env::current_exe().unwrap())
        .args(["--exact", &test, "--nocapture"])
        .env(ROOT_ENV, root)
        .env(CASE_ENV, case)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let output = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let ready = BufReader::new(output).lines().find_map(|line| {
            line.ok()?
                .split_once(READY)
                .map(|(_, ready)| ready.to_owned())
        });
        let _ = sender.send(ready);
    });
    let ready = receiver.recv_timeout(Duration::from_secs(10));
    // Reap the child even when it failed before reaching the handshake.
    let killed = child.kill();
    let status = child.wait().unwrap();
    reader.join().unwrap();
    assert!(killed.is_ok(), "{case}: child exited before kill: {status}");
    assert!(!status.success(), "{case}: expected forced termination");
    let ready = ready
        .expect("bounded crash boundary")
        .expect("child reached boundary");
    let (generation, usage) = ready.split_once(' ').unwrap();
    (generation.into(), usage.parse().unwrap())
}

#[test]
fn catalog_recovers_after_forced_process_exit() {
    if let Some(root) = std::env::var_os(ROOT_ENV) {
        crash_child(root.into(), &std::env::var(CASE_ENV).unwrap());
        return;
    }
    for case in [
        "committed",
        "write",
        "prepared-write",
        "upgrade",
        "unpublished",
        "marker",
        "allocated",
        "creating-synced",
        "deleted",
        "renamed",
        "source-synced",
    ] {
        let directory = Directory::new();
        let root = directory.0.join("idb-lite");
        let (generation, usage) = kill_at_boundary(&root, case);
        let service = Service::on_disk(root.clone(), config()).unwrap();
        if matches!(case, "committed" | "write" | "prepared-write" | "upgrade") {
            assert_eq!(service.database_version(OWNER, NAME).unwrap(), Some(1));
            assert_eq!(service.owner_usage_bytes(OWNER).unwrap(), usage);
            let client = resolve(&service, OWNER, NAME);
            assert_eq!(
                moli_crypto::sha256_hex(client.identity().generation),
                generation
            );
            let metadata = client.committed().unwrap();
            assert_eq!(
                metadata
                    .stores
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                ["a", "b"]
            );
            verify_record(&client);
            let lease = queue(&client, "a", TransactionMode::ReadWrite);
            start(&client, "a", TransactionMode::ReadWrite, &lease);
            let key = execute(
                &client,
                Command::Write {
                    transaction: TX,
                    store: "a".into(),
                    key: None,
                    value: b"next".to_vec().into(),
                    indexes: vec![],
                    add_only: true,
                    quota: None,
                },
            )
            .unwrap();
            assert!(matches!(key, CommandResult::Key(key) if key == Key::from(2)));
            execute(&client, Command::Abort { transaction: TX }).unwrap();
        } else {
            assert!(service.databases(OWNER).unwrap().is_empty());
            assert_eq!(service.owner_usage_bytes(OWNER).unwrap(), 0);
            let client = resolve(&service, OWNER, NAME);
            assert_eq!(client.committed().unwrap().version, 0);
            let recovered = moli_crypto::sha256_hex(client.identity().generation);
            assert_eq!(recovered == generation, case == "unpublished");
            seed(&client);
            verify_record(&client);
        }
        assert_eq!(fs::read_dir(root.join("trash")).unwrap().count(), 0);
    }
}
