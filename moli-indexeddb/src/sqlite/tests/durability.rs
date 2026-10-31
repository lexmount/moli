use super::*;
use std::{cell::Cell, cell::RefCell, path::PathBuf, ptr::NonNull, rc::Rc};

struct Disk(PathBuf);

impl Disk {
    fn new() -> Self {
        let mut token = [0; 16];
        moli_crypto::fill_secure_random(&mut token).unwrap();
        let path =
            std::env::temp_dir().join(format!("moli-idb-sync-{}", moli_crypto::sha256_hex(token)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn open(&self, create: bool) -> Database {
        let connection = moli_sqlite::open_disk(
            &self.0.join("db.sqlite"),
            if create {
                moli_sqlite::OpenMode::Create
            } else {
                moli_sqlite::OpenMode::Existing
            },
            Default::default(),
        )
        .unwrap();
        if create {
            Database::create(connection, identity()).unwrap()
        } else {
            Database::open(connection, identity()).unwrap()
        }
    }
}

impl Drop for Disk {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct WalIoState {
    original: rusqlite::ffi::sqlite3_io_methods,
    syncs: Cell<usize>,
    fail_sync: Cell<bool>,
    fail_write: Cell<bool>,
}

thread_local! {
    // Each test owns a connection on its own thread. No global VFS override or
    // fault flag can affect another test running concurrently.
    static WAL_IO: RefCell<BTreeMap<usize, Rc<WalIoState>>> = const { RefCell::new(BTreeMap::new()) };
}

struct WalIoProbe {
    file: NonNull<rusqlite::ffi::sqlite3_file>,
    original: *const rusqlite::ffi::sqlite3_io_methods,
    _methods: Box<rusqlite::ffi::sqlite3_io_methods>,
    state: Rc<WalIoState>,
}

impl WalIoProbe {
    // The caller keeps this WAL connection open until the probe is dropped.
    // SQLITE_FCNTL_JOURNAL_POINTER returns the already-open WAL file. Only its
    // xSync/xWrite callbacks are replaced, retaining all real platform I/O.
    unsafe fn install(connection: &Connection) -> Self {
        let mut file = std::ptr::null_mut::<rusqlite::ffi::sqlite3_file>();
        let result = unsafe {
            rusqlite::ffi::sqlite3_file_control(
                connection.handle(),
                c"main".as_ptr(),
                rusqlite::ffi::SQLITE_FCNTL_JOURNAL_POINTER,
                (&mut file as *mut *mut rusqlite::ffi::sqlite3_file).cast(),
            )
        };
        assert_eq!(result, rusqlite::ffi::SQLITE_OK);
        let mut file = NonNull::new(file).expect("seeded disk database has an open WAL");
        let original = unsafe { file.as_ref().pMethods };
        assert!(!original.is_null());
        let original_methods = unsafe { *original };
        assert!(original_methods.xSync.is_some() && original_methods.xWrite.is_some());
        let state = Rc::new(WalIoState {
            original: original_methods,
            syncs: Cell::new(0),
            fail_sync: Cell::new(false),
            fail_write: Cell::new(false),
        });
        let methods = Box::new(rusqlite::ffi::sqlite3_io_methods {
            xSync: Some(observe_sync),
            xWrite: Some(observe_write),
            ..original_methods
        });
        WAL_IO.with(|map| {
            assert!(
                map.borrow_mut()
                    .insert(file.as_ptr() as usize, state.clone())
                    .is_none()
            );
        });
        unsafe { file.as_mut().pMethods = &*methods };
        Self {
            file,
            original,
            _methods: methods,
            state,
        }
    }
}

impl Drop for WalIoProbe {
    fn drop(&mut self) {
        // The owning test drops this guard before closing its connection.
        unsafe { self.file.as_mut().pMethods = self.original };
        WAL_IO.with(|map| map.borrow_mut().remove(&(self.file.as_ptr() as usize)));
    }
}

unsafe extern "C" fn observe_sync(file: *mut rusqlite::ffi::sqlite3_file, flags: i32) -> i32 {
    let state = WAL_IO.with(|map| map.borrow().get(&(file as usize)).cloned());
    let Some(state) = state else {
        return rusqlite::ffi::SQLITE_IOERR_FSYNC;
    };
    state.syncs.set(state.syncs.get() + 1);
    if state.fail_sync.replace(false) {
        return rusqlite::ffi::SQLITE_IOERR_FSYNC;
    }
    match state.original.xSync {
        Some(sync) => unsafe { sync(file, flags) },
        None => rusqlite::ffi::SQLITE_IOERR_FSYNC,
    }
}

unsafe extern "C" fn observe_write(
    file: *mut rusqlite::ffi::sqlite3_file,
    data: *const std::ffi::c_void,
    size: i32,
    offset: i64,
) -> i32 {
    let state = WAL_IO.with(|map| map.borrow().get(&(file as usize)).cloned());
    let Some(state) = state else {
        return rusqlite::ffi::SQLITE_IOERR_WRITE;
    };
    if state.fail_write.replace(false) {
        return rusqlite::ffi::SQLITE_IOERR_WRITE;
    }
    match state.original.xWrite {
        Some(write) => unsafe { write(file, data, size, offset) },
        None => rusqlite::ffi::SQLITE_IOERR_WRITE,
    }
}

fn seed(database: &mut Database) {
    database
        .connection
        .pragma_update(None, "wal_autocheckpoint", 0)
        .unwrap();
    database
        .begin(TX, TransactionMode::VersionChange, BTreeSet::new(), Some(1))
        .unwrap();
    for name in ["a", "b"] {
        database
            .create_object_store(
                TX,
                name,
                ObjectStoreOptions {
                    auto_increment: true,
                    ..Default::default()
                },
            )
            .unwrap();
    }
    put(database, None, b"seed", vec![], None).unwrap();
    database.commit(TX).unwrap();
}

fn begin_write(database: &mut Database, durability: TransactionDurability) {
    database
        .begin_with_durability(
            TX,
            TransactionMode::ReadWrite,
            BTreeSet::from(["a".into()]),
            None,
            durability,
        )
        .unwrap();
}

#[test]
fn sqlite_durability_selects_real_wal_sync_without_affecting_readers() {
    let disk = Disk::new();
    let mut database = disk.open(true);
    seed(&mut database);
    // Safety: this guard is explicitly dropped before the connection closes.
    let io = unsafe { WalIoProbe::install(&database.connection) };
    for (index, durability) in [
        TransactionDurability::Strict,
        TransactionDurability::Relaxed,
        TransactionDurability::Strict,
        TransactionDurability::Relaxed,
    ]
    .into_iter()
    .enumerate()
    {
        begin_write(&mut database, durability);
        let expected = if durability == TransactionDurability::Strict {
            2
        } else {
            1
        };
        let level = || {
            database
                .connection
                .pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0))
                .unwrap()
        };
        assert_eq!(level(), expected);
        let reader = TransactionHandle::from_raw(2);
        database
            .begin_with_durability(
                reader,
                TransactionMode::ReadOnly,
                BTreeSet::from(["b".into()]),
                None,
                if durability == TransactionDurability::Strict {
                    TransactionDurability::Relaxed
                } else {
                    TransactionDurability::Strict
                },
            )
            .unwrap();
        database
            .commit_with_quota(
                reader,
                Some(QuotaCheck {
                    quota: 0,
                    other_usage: u64::MAX,
                }),
            )
            .unwrap();
        assert_eq!(
            database
                .connection
                .pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0))
                .unwrap(),
            expected
        );
        assert_eq!(
            put(&mut database, None, &[index as u8], vec![], None).unwrap(),
            ((index + 2) as i64).into()
        );
        io.state.syncs.set(0);
        database.commit(TX).unwrap();
        assert_eq!(
            io.state.syncs.get() > 0,
            durability == TransactionDurability::Strict
        );
    }
    drop(io);
    database.close().unwrap();
    let mut reopened = disk.open(false);
    begin_write(&mut reopened, TransactionDurability::Strict);
    assert_eq!(reopened.count(TX, "a").unwrap(), 5);
    assert_eq!(
        put(&mut reopened, None, b"next", vec![], None).unwrap(),
        6.into()
    );
    reopened.abort(TX).unwrap();
}

#[test]
fn sqlite_durability_io_failure_rolls_back_records_schema_version_and_generator() {
    for (durability, fail_sync) in [
        (TransactionDurability::Relaxed, false),
        (TransactionDurability::Strict, false),
        (TransactionDurability::Strict, true),
    ] {
        for upgrade in [false, true] {
            let disk = Disk::new();
            let mut database = disk.open(true);
            seed(&mut database);
            let committed = database.committed().clone();
            // Safety: no close or journal-mode change occurs before dropping io.
            let io = unsafe { WalIoProbe::install(&database.connection) };
            if upgrade {
                database
                    .begin_with_durability(
                        TX,
                        TransactionMode::VersionChange,
                        BTreeSet::new(),
                        Some(2),
                        durability,
                    )
                    .unwrap();
                database
                    .create_object_store(TX, "uncommitted", ObjectStoreOptions::default())
                    .unwrap();
            } else {
                begin_write(&mut database, durability);
            }
            assert_eq!(
                put(&mut database, None, b"discarded", vec![], None).unwrap(),
                2.into()
            );
            if fail_sync {
                io.state.fail_sync.set(true);
            } else {
                io.state.fail_write.set(true);
            }
            assert!(matches!(database.commit(TX), Err(IndexedDbError::Io(_))));
            assert!(!io.state.fail_sync.get() && !io.state.fail_write.get());
            assert!(!database.is_transaction_active());
            assert!(Arc::ptr_eq(&committed, database.committed()));
            assert!(database.commit(TX).is_err());
            begin_write(&mut database, durability);
            assert_eq!(database.get(TX, "a", &2.into()).unwrap(), None);
            assert_eq!(
                put(&mut database, None, b"next", vec![], None).unwrap(),
                2.into()
            );
            database.abort(TX).unwrap();
            drop(io);
            database.close().unwrap();
            let mut reopened = disk.open(false);
            assert_eq!(reopened.committed().version, 1);
            assert_eq!(reopened.committed().stores.len(), 2);
            begin_write(&mut reopened, durability);
            assert_eq!(reopened.get(TX, "a", &2.into()).unwrap(), None);
            assert_eq!(
                put(&mut reopened, None, b"next", vec![], None).unwrap(),
                2.into()
            );
            reopened.abort(TX).unwrap();
        }
    }
}

#[test]
fn sqlite_durability_quota_failure_never_publishes_a_write() {
    let disk = Disk::new();
    let mut database = disk.open(true);
    seed(&mut database);
    for durability in [
        TransactionDurability::Relaxed,
        TransactionDurability::Strict,
    ] {
        let committed = database.committed().clone();
        begin_write(&mut database, durability);
        put(&mut database, None, b"over quota", vec![], None).unwrap();
        assert!(matches!(
            database.commit_with_quota(
                TX,
                Some(QuotaCheck {
                    quota: committed.usage,
                    other_usage: 0,
                })
            ),
            Err(IndexedDbError::QuotaExceeded { .. })
        ));
        assert!(Arc::ptr_eq(&committed, database.committed()));
        begin_write(&mut database, durability);
        assert_eq!(
            put(&mut database, None, b"next", vec![], None).unwrap(),
            2.into()
        );
        database.abort(TX).unwrap();
    }
}
