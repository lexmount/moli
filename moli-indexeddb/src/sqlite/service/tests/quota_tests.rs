use super::*;

fn reject(client: &DatabaseClient, command: Command) -> Completion {
    let (sender, receiver) = mpsc::channel();
    client
        .submit_checked(
            command,
            Err(IndexedDbError::InvalidState(
                "bucket is no longer current".into(),
            )),
            move |reply| {
                let _ = sender.send(reply);
            },
        )
        .unwrap();
    receive(&receiver)
}

fn write() -> Command {
    Command::Write {
        transaction: TX,
        store: "a".into(),
        key: None,
        value: b"kept".to_vec().into(),
        indexes: vec![],
        add_only: false,
        quota: None,
    }
}

#[test]
fn quota_preflight_failure_reconciles_prepared_and_atomic_requests_and_commit() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "quota-preflight", Location::Memory);
    initialize(&client);
    let original = client.committed().unwrap();
    let lease = queue(&client, "a", TransactionMode::ReadWrite);
    start(&client, "a", TransactionMode::ReadWrite, &lease);
    let failed = reject(&client, write());
    assert!(matches!(
        failed.result,
        Err(IndexedDbError::InvalidState(_))
    ));
    assert_eq!(failed.active_transaction, Some(TX));
    drop(failed);
    let CommandResult::Prepared(prepared) = execute(
        &client,
        Command::PrepareWrite {
            transaction: TX,
            store: "a".into(),
            add_only: false,
        },
    )
    .unwrap() else {
        panic!("prepared write");
    };
    assert_eq!(prepared.key, 1.into());
    let failed = reject(
        &client,
        Command::FinishWrite {
            transaction: TX,
            request: prepared.id,
            value: b"discarded".to_vec().into(),
            indexes: vec![],
            quota: None,
        },
    );
    assert!(matches!(
        failed.result,
        Err(IndexedDbError::InvalidState(_))
    ));
    assert_eq!(failed.active_transaction, Some(TX));
    drop(failed);
    let CommandResult::Key(key) = execute(&client, write()).unwrap() else {
        panic!("written key");
    };
    assert_eq!(key, 1.into());
    let failed = reject(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    );
    assert!(matches!(
        failed.result,
        Err(IndexedDbError::InvalidState(_))
    ));
    assert_eq!(failed.active_transaction, None);
    assert_eq!(*client.committed().unwrap(), *original);
    let next = queue(&client, "a", TransactionMode::ReadWrite);
    assert!(next.handle().is_ready());
    start(&client, "a", TransactionMode::ReadWrite, &next);
    let CommandResult::Key(key) = execute(&client, write()).unwrap() else {
        panic!("written key");
    };
    assert_eq!(key, 1.into());
    execute(
        &client,
        Command::Commit {
            transaction: TX,
            quota: None,
        },
    )
    .unwrap();
}
