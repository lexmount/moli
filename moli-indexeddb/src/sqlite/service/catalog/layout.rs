//! Durable directory publication. The profile owns the process lock; runtime
//! generation preparation and retirement run on the assigned SQL worker.
use super::*;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
const FORMAT: &[u8] = b"{\"backend\":\"indexeddb\",\"layout\":1}\n";
const PREPARING: &str = ".creating-";

#[cfg(test)]
type BoundaryHook = (Boundary, Box<dyn FnOnce() + Send>);

pub(in crate::sqlite::service) struct DiskCatalog {
    root: PathBuf,
    #[cfg(test)]
    fault: Mutex<Option<Boundary>>,
    #[cfg(test)]
    hook: Mutex<Option<BoundaryHook>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::sqlite::service) enum Boundary {
    Allocated,
    MarkerSynced,
    CreatingSynced,
    Retiring,
    Renamed,
    SourceSynced,
}

impl DiskCatalog {
    pub(super) fn open(root: PathBuf) -> Result<Self, IndexedDbError> {
        let root = if root.is_absolute() {
            root
        } else {
            std::env::current_dir()
                .map_err(|error| io_error("resolve root", &root, error))?
                .join(root)
        };
        ensure_directory(&root)?;
        // Rust's filesystem APIs support long Windows paths implicitly, but
        // SQLite's Win32 VFS needs the verbatim prefix. Resolve the existing
        // root once so generation paths and recovery use the same identity.
        #[cfg(windows)]
        let root =
            fs::canonicalize(&root).map_err(|error| io_error("canonicalize root", &root, error))?;
        sync_directory(
            root.parent()
                .ok_or_else(|| corruption("IndexedDB root has no parent"))?,
        )?;
        let format = root.join("format.json");
        match fs::symlink_metadata(&format) {
            Ok(metadata) => {
                if !metadata.is_file() || metadata.len() > 4096 {
                    return Err(corruption("invalid IndexedDB root format"));
                }
                let mut data = Vec::new();
                File::open(&format)
                    .and_then(|file| file.take(4097).read_to_end(&mut data))
                    .map_err(|error| io_error("read format", &format, error))?;
                if data != FORMAT {
                    return Err(corruption("unsupported IndexedDB root format"));
                }
                require_directory(&root.join("live"))?;
                require_directory(&root.join("trash"))?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                for entry in read_directory(&root)? {
                    let entry = entry
                        .map_err(|error| io_error("inspect unformatted root", &root, error))?;
                    let name = entry.file_name();
                    let recognized = name == "live"
                        || name == "trash"
                        || name.to_str().is_some_and(|name| {
                            name.strip_prefix(".format-")
                                .and_then(|name| name.strip_suffix(".tmp"))
                                .is_some_and(|token| is_hex(token, 32))
                        });
                    if !recognized {
                        return Err(corruption(
                            "unrecognized contents in unformatted IndexedDB root",
                        ));
                    }
                }
                for name in ["live", "trash"] {
                    let directory = root.join(name);
                    ensure_directory(&directory)?;
                    if read_directory(&directory)?.next().is_some() {
                        return Err(corruption(
                            "IndexedDB root format is missing from nonempty storage",
                        ));
                    }
                }
                // The marker cannot become authoritative before the empty
                // live/trash directories are themselves durable.
                sync_directory(&root)?;
                let temporary = root.join(format!(".format-{}.tmp", token()?));
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)
                    .map_err(|error| io_error("create format", &temporary, error))?;
                file.write_all(FORMAT)
                    .and_then(|()| file.sync_all())
                    .map_err(|error| io_error("sync format", &temporary, error))?;
                fs::rename(&temporary, &format)
                    .map_err(|error| io_error("publish format", &format, error))?;
            }
            Err(error) => return Err(io_error("inspect format", &format, error)),
        }
        sync_directory(&root)?;
        Ok(Self {
            root,
            #[cfg(test)]
            fault: Mutex::new(None),
            #[cfg(test)]
            hook: Mutex::new(None),
        })
    }

    pub(super) fn path(
        &self,
        owner: &[u8],
        name: &crate::IndexedDbName,
        generation: [u8; 16],
    ) -> PathBuf {
        self.root
            .join("live")
            .join(hash(1, owner))
            .join(hash_database_name(name))
            .join(hex(&generation))
            .join("db.sqlite")
    }

    pub(super) fn verify(
        &self,
        identity: &DatabaseIdentity,
        path: &Path,
    ) -> Result<(), IndexedDbError> {
        if identity.owner_key.is_empty()
            || path != self.path(&identity.owner_key, &identity.name, identity.generation)
        {
            return Err(corruption(
                "IndexedDB generation path does not match its identity",
            ));
        }
        Ok(())
    }

    pub(in crate::sqlite::service) fn prepare(
        &self,
        path: &Path,
        identity: &DatabaseIdentity,
    ) -> Result<(), IndexedDbError> {
        self.verify(identity, path)?;
        let generation = path.parent().expect("generation path");
        let name = generation.parent().expect("name path");
        let owner = name.parent().expect("owner path");
        ensure_directory(owner)?;
        sync_directory(owner.parent().expect("live root"))?;
        ensure_directory(name)?;
        sync_directory(owner)?;
        // Persist intent before mkdir. An empty unmarked directory then always
        // means missing/corrupt storage, never a safely disposable creation.
        let marker = preparation_marker(generation);
        let retry = preparing(generation)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(!retry)
            .open(&marker)
            .map_err(|error| io_error("create preparation marker", &marker, error))?;
        file.write_all(hex(&identity.generation).as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| io_error("sync preparation marker", &marker, error))?;
        sync_directory(name)?;
        self.boundary(Boundary::MarkerSynced)?;
        match fs::create_dir(generation) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if !retry {
                    return Err(corruption("unmarked IndexedDB generation already exists"));
                }
                require_directory(generation)?;
            }
            Err(error) => return Err(io_error("allocate generation", generation, error)),
        }
        sync_directory(name)?;
        self.boundary(Boundary::Allocated)
    }

    pub(in crate::sqlite::service) fn finish_preparation(
        &self,
        path: &Path,
        connection: &Connection,
    ) -> Result<(), IndexedDbError> {
        let busy: i64 =
            connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
        if busy != 0 {
            return Err(corruption("creating IndexedDB checkpoint is busy"));
        }
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .and_then(|file| file.sync_all())
            .map_err(|error| io_error("sync creating database", path, error))?;
        let generation = path.parent().expect("generation path");
        sync_directory(generation)?;
        self.boundary(Boundary::CreatingSynced)?;
        match fs::remove_file(preparation_marker(generation)) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(io_error("remove preparation marker", generation, error)),
        }
        sync_directory(generation.parent().expect("name path"))
    }

    pub(in crate::sqlite::service) fn retire(&self, path: &Path) -> Result<(), IndexedDbError> {
        self.boundary(Boundary::Retiring)?;
        let generation = path
            .parent()
            .ok_or_else(|| corruption("missing generation directory"))?;
        let parent = generation
            .parent()
            .ok_or_else(|| corruption("missing generation parent"))?;
        let trash = self.root.join("trash");
        let slot = trash.join(token()?);
        fs::create_dir(&slot).map_err(|error| io_error("allocate trash slot", &slot, error))?;
        sync_directory(&trash)?;
        // A newly allocated private parent makes payload an unused destination,
        // avoiding rename's overwrite behavior on existing empty directories.
        fs::rename(generation, slot.join("payload"))
            .map_err(|error| io_error("retire generation", generation, error))?;
        self.boundary(Boundary::Renamed)?;
        sync_directory(parent)?;
        self.boundary(Boundary::SourceSynced)?;
        sync_directory(&slot)?;
        // Logical deletion is durable. Physical cleanup is retryable.
        let _ = fs::remove_dir_all(&slot);
        Ok(())
    }

    pub(super) fn visit(
        &self,
        mut visit: impl FnMut(PathBuf, bool) -> Result<(), IndexedDbError>,
    ) -> Result<(), IndexedDbError> {
        for owner in read_directory(&self.root.join("live"))? {
            let owner = directory_entry(owner, 64)?;
            for name in read_directory(&owner)? {
                let name = directory_entry(name, 64)?;
                for entry in read_directory(&name)? {
                    let entry =
                        entry.map_err(|error| io_error("read generation entry", &name, error))?;
                    if let Some(generation) = entry
                        .file_name()
                        .to_str()
                        .and_then(|name| name.strip_prefix(PREPARING))
                    {
                        if !is_hex(generation, 32) {
                            return Err(corruption("invalid preparation name"));
                        }
                        let directory = name.join(generation);
                        if !directory
                            .try_exists()
                            .map_err(|error| io_error("inspect generation", &directory, error))?
                            && preparing(&directory)?
                        {
                            fs::remove_file(preparation_marker(&directory)).map_err(|error| {
                                io_error("remove unused preparation marker", &directory, error)
                            })?;
                            sync_directory(&name)?;
                        }
                        continue;
                    }
                    let generation = directory_entry(Ok(entry), 32)?;
                    let preparing = preparing(&generation)?;
                    let path = generation.join("db.sqlite");
                    if path
                        .try_exists()
                        .map_err(|error| io_error("inspect database", &path, error))?
                        && !fs::symlink_metadata(&path)
                            .map_err(|error| io_error("inspect database", &path, error))?
                            .is_file()
                    {
                        return Err(corruption("IndexedDB database is not a regular file"));
                    }
                    visit(path, preparing)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn discard_preparing(&self, path: &Path) -> Result<(), IndexedDbError> {
        let generation = path.parent().expect("generation path");
        let name = generation.parent().expect("name path");
        fs::remove_dir_all(generation)
            .map_err(|error| io_error("discard unpublished generation", generation, error))?;
        // Persist removal before clearing intent, so interrupted recovery
        // cannot resurrect an unmarked empty generation.
        sync_directory(name)?;
        fs::remove_file(preparation_marker(generation))
            .map_err(|error| io_error("remove preparation marker", generation, error))?;
        sync_directory(name)
    }

    pub(super) fn cleanup_trash(&self) {
        let trash = self.root.join("trash");
        if let Ok(entries) = fs::read_dir(&trash) {
            for entry in entries.flatten() {
                if let Ok(path) = directory_entry(Ok(entry), 32) {
                    let _ = fs::remove_dir_all(path);
                }
            }
            let _ = sync_directory(&trash);
        }
    }

    fn boundary(&self, point: Boundary) -> Result<(), IndexedDbError> {
        #[cfg(test)]
        {
            let hook = {
                let mut hook = self.hook.lock();
                if hook
                    .as_ref()
                    .is_some_and(|(boundary, _)| *boundary == point)
                {
                    hook.take()
                } else {
                    None
                }
            };
            if let Some((_, hook)) = hook {
                hook();
            }
            let mut fault = self.fault.lock();
            if *fault == Some(point) {
                *fault = None;
                return Err(IndexedDbError::Io(
                    "injected IndexedDB catalog interruption".into(),
                ));
            }
        }
        let _ = point;
        Ok(())
    }

    #[cfg(test)]
    pub(in crate::sqlite::service) fn interrupt_at(&self, point: Boundary) {
        *self.fault.lock() = Some(point);
    }

    #[cfg(test)]
    pub(in crate::sqlite::service) fn on_boundary(
        &self,
        point: Boundary,
        hook: impl FnOnce() + Send + 'static,
    ) {
        *self.hook.lock() = Some((point, Box::new(hook)));
    }
}

fn preparation_marker(generation: &Path) -> PathBuf {
    generation.parent().expect("name path").join(format!(
        "{PREPARING}{}",
        generation
            .file_name()
            .expect("generation name")
            .to_string_lossy()
    ))
}

fn preparing(generation: &Path) -> Result<bool, IndexedDbError> {
    let marker = preparation_marker(generation);
    match fs::symlink_metadata(&marker) {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.len() != 32 {
                return Err(corruption("invalid preparation marker"));
            }
            let mut data = Vec::new();
            File::open(&marker)
                .and_then(|file| file.take(33).read_to_end(&mut data))
                .map_err(|error| io_error("read preparation marker", &marker, error))?;
            if data
                != generation
                    .file_name()
                    .expect("generation name")
                    .to_string_lossy()
                    .as_bytes()
            {
                return Err(corruption("preparation identity mismatch"));
            }
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error("inspect preparation marker", &marker, error)),
    }
}

fn is_hex(name: &str, length: usize) -> bool {
    name.len() == length
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn directory_entry(
    entry: std::io::Result<fs::DirEntry>,
    length: usize,
) -> Result<PathBuf, IndexedDbError> {
    let entry =
        entry.map_err(|error| IndexedDbError::Io(format!("IndexedDB directory entry: {error}")))?;
    if !entry
        .file_type()
        .map_err(|error| io_error("inspect directory", &entry.path(), error))?
        .is_dir()
        || !entry
            .file_name()
            .to_str()
            .is_some_and(|name| is_hex(name, length))
    {
        return Err(corruption("invalid IndexedDB catalog directory"));
    }
    Ok(entry.path())
}

fn read_directory(path: &Path) -> Result<fs::ReadDir, IndexedDbError> {
    fs::read_dir(path).map_err(|error| io_error("read directory", path, error))
}

fn ensure_directory(path: &Path) -> Result<(), IndexedDbError> {
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if fs::symlink_metadata(path)
                .map_err(|error| io_error("inspect directory", path, error))?
                .is_dir()
            {
                Ok(())
            } else {
                Err(corruption("IndexedDB catalog directory is not a directory"))
            }
        }
        Err(error) => Err(io_error("create directory", path, error)),
    }
}

fn require_directory(path: &Path) -> Result<(), IndexedDbError> {
    if !fs::symlink_metadata(path)
        .map_err(|error| io_error("inspect directory", path, error))?
        .is_dir()
    {
        return Err(corruption("IndexedDB catalog directory is not a directory"));
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), IndexedDbError> {
    #[cfg(not(windows))]
    let result = File::open(path).and_then(|file| file.sync_all());
    #[cfg(windows)]
    let result = {
        use std::os::windows::fs::OpenOptionsExt;
        OpenOptions::new()
            .access_mode(0x4000_0000)
            .custom_flags(0x0200_0000)
            .open(path)
            .and_then(|file| file.sync_all())
    };
    result.map_err(|error| io_error("sync directory", path, error))
}

fn token() -> Result<String, IndexedDbError> {
    let mut bytes = [0; 16];
    moli_crypto::fill_secure_random(&mut bytes)
        .map_err(|error| IndexedDbError::Io(error.to_string()))?;
    Ok(hex(&bytes))
}

fn hash(tag: u8, bytes: &[u8]) -> String {
    let mut hash = moli_crypto::Sha256Context::new();
    hash.update(b"moli-indexeddb-path-v1\0");
    hash.update([tag]);
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
    hex(&hash.finish())
}

pub(super) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(result, "{byte:02x}");
    }
    result
}

pub(super) fn io_error(action: &str, path: &Path, error: std::io::Error) -> IndexedDbError {
    IndexedDbError::Io(format!("IndexedDB {action} `{}`: {error}", path.display()))
}

/// Keep every schema-v1 Unicode name in its original generation directory.
/// Canonical WTF-8 extends the same hash domain to unpaired surrogates without
/// projecting distinct DOMStrings onto the same replacement character.
fn hash_database_name(name: &crate::IndexedDbName) -> String {
    let mut hash = moli_crypto::Sha256Context::new();
    hash.update(b"moli-indexeddb-path-v1\0");
    hash.update([2]);
    hash.update(name.usage_bytes().to_be_bytes());
    for character in char::decode_utf16(name.as_utf16().iter().copied()) {
        match character {
            Ok(character) => {
                let mut bytes = [0; 4];
                hash.update(character.encode_utf8(&mut bytes).as_bytes());
            }
            Err(error) => {
                let surrogate = error.unpaired_surrogate();
                hash.update([
                    0xe0 | (surrogate >> 12) as u8,
                    0x80 | ((surrogate >> 6) & 0x3f) as u8,
                    0x80 | (surrogate & 0x3f) as u8,
                ]);
            }
        }
    }
    hex(&hash.finish())
}

#[cfg(test)]
mod database_name_tests {
    use super::*;

    #[test]
    fn database_name_paths_keep_legacy_unicode_hashes_and_distinguish_surrogates() {
        for text in [
            "",
            "\0",
            "ordinary",
            "\u{1f4be}",
            "\u{e000}",
            "legacy-\0-数据库-\u{1f4be}",
        ] {
            assert_eq!(hash_database_name(&text.into()), hash(2, text.as_bytes()));
        }
        let names = [
            vec![0xd800],
            vec![0xd801],
            vec![0xfffd],
            vec![0x64, 0x38, 0x30, 0x30],
            vec![0xdfff, 0xd800],
            vec![0xfffd, 0xfffd],
        ];
        let hashes = names
            .into_iter()
            .map(|units| hash_database_name(&crate::IndexedDbName::from_utf16(units)))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(hashes.len(), 6);
    }
}
