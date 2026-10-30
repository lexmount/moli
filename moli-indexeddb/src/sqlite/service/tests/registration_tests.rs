use super::*;

#[test]
fn existing_registration_reuses_identity_and_committed_snapshot() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "same", Location::Memory);
    initialize(&client);
    let original = client.committed().unwrap();
    let identity = client.identity.clone();
    with_database(&client, move |database| {
        assert!(Arc::ptr_eq(&identity, &database.identity))
    });
    let (reopened, result) = register(&service, "same", Location::Memory);
    assert_eq!(reopened.id(), client.id());
    assert!(Arc::ptr_eq(&reopened.identity, &client.identity));
    let CommandResult::Metadata(snapshot) = result.result.unwrap() else {
        panic!("expected metadata")
    };
    assert!(Arc::ptr_eq(&original, &snapshot));
}

#[test]
fn registration_identity_is_released_independently_of_retained_schema() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "old", Location::Memory);
    let held_snapshot = client.committed().unwrap();
    let identity = Arc::downgrade(&client.identity);
    execute(&client, Command::Close).unwrap();
    let other = open(&service, "other", Location::Memory);
    // Opening prunes the old closed registry entry; the old client still owns
    // its identity. The old schema must not retain that registration.
    assert_eq!(service.registry.lock().entries.len(), 1);
    drop(client);
    assert!(identity.upgrade().is_none());
    assert_eq!(held_snapshot.name, crate::IndexedDbName::from("old"));
    drop(held_snapshot);
    drop(other);
}
