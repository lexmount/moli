use super::*;

mod directory_tests {
    use super::*;
    fn selected_files_from_paths(paths: &[String]) -> Result<Vec<SelectedFile>, String> {
        super::selected_files_from_paths(paths).map_err(|error| error.to_string())
    }

    #[tokio::test]
    async fn selected_directory_preserves_relative_paths_contents_and_empty_directories() {
        let scratch = std::env::temp_dir().join(format!(
            "moli-directory-upload-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = scratch.join("chosen");
        fs::create_dir_all(root.join("nested/empty")).unwrap();
        fs::write(root.join("a.txt"), b"first\0\xff").unwrap();
        fs::write(root.join("nested/b.txt"), b"second").unwrap();
        let selected = selected_files_from_paths(&[root.to_string_lossy().into_owned()]).unwrap();
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].relative_path, "chosen/a.txt");
        assert_eq!(selected[0].name, "a.txt");
        assert_eq!(selected[0].data.read_blocking().unwrap(), b"first\0\xff");
        assert_eq!(selected[1].relative_path, "chosen/nested/b.txt");
        assert_eq!(selected[1].data.read_blocking().unwrap(), b"second");
        let ordinary =
            selected_files_from_paths(&[root.join("a.txt").to_string_lossy().into_owned()])
                .unwrap();
        assert!(ordinary[0].relative_path.is_empty());
        assert!(
            selected_files_from_paths(&[root.join("nested/empty").to_string_lossy().into_owned()])
                .unwrap()
                .is_empty()
        );
        assert!(
            selected_files_from_paths(&[root.join("missing").to_string_lossy().into_owned()])
                .is_err()
        );
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn upload_relative_paths_use_the_selected_directory_name() {
        let current = std::env::current_dir().unwrap();
        let root = upload_directory_root(Path::new(".")).unwrap();
        assert_eq!(root, fs::canonicalize(&current).unwrap());
        assert_eq!(
            upload_relative_path(root.file_name(), Path::new("nested/file.txt")),
            format!(
                "{}/nested/file.txt",
                root.file_name().unwrap().to_string_lossy()
            )
        );
        let parent = upload_directory_root(&current.join("..")).unwrap();
        assert_eq!(parent, fs::canonicalize(current.join("..")).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn upload_from_a_filesystem_root_does_not_emit_an_absolute_relative_path() {
        // Exercise path derivation without enumerating the machine's root.
        assert_eq!(
            upload_relative_path(Path::new("/").file_name(), Path::new("file.txt")),
            "file.txt"
        );
        assert_eq!(
            upload_relative_path(Path::new("/").file_name(), Path::new("nested/file.txt")),
            "nested/file.txt"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn explicit_directory_symlink_is_followed_but_descendant_links_are_skipped() {
        use std::os::unix::fs::symlink;
        let scratch = std::env::temp_dir().join(format!(
            "moli-directory-links-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = scratch.join("chosen");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::create_dir_all(scratch.join("outside")).unwrap();
        fs::write(root.join("nested/real.txt"), b"real").unwrap();
        fs::write(scratch.join("outside/secret.txt"), b"outside").unwrap();
        symlink(scratch.join("outside"), root.join("directory-link")).unwrap();
        symlink(root.join("nested/real.txt"), root.join("file-link")).unwrap();
        symlink(&root, scratch.join("selected-link")).unwrap();
        let selected = selected_files_from_paths(&[scratch
            .join("selected-link")
            .to_string_lossy()
            .into_owned()])
        .unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].relative_path, "selected-link/nested/real.txt");
        assert_eq!(selected[0].data.read_blocking().unwrap(), b"real");
        fs::remove_dir_all(scratch).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn non_unicode_upload_name_never_falls_back_to_the_absolute_path() {
        use std::os::unix::ffi::OsStringExt;
        let scratch = std::env::temp_dir().join(format!(
            "moli-directory-name-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&scratch).unwrap();
        let file = scratch.join(std::ffi::OsString::from_vec(b"bad-\xff.txt".to_vec()));
        fs::write(&file, b"data").unwrap();
        let selected = selected_file_from_file_path(
            &file,
            &tokio::runtime::Handle::current(),
            FileReadPolicy::ExplicitFile,
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(selected.name, "bad-\u{fffd}.txt");
        assert!(selected.relative_path.is_empty());
        fs::remove_dir_all(scratch).unwrap();
    }
}

mod backing_tests {
    use super::*;
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(label: &str) -> Self {
            let directory = std::env::temp_dir().join(format!(
                "moli-file-{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&directory).unwrap();
            Self(directory)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn file_selection_defers_reads_and_shares_immutable_ranges() {
        let directory = Scratch::new("backing");
        let path = directory.0.join("file.txt");
        fs::write(&path, b"abcdef").unwrap();
        let selected = selected_files_from_paths(&[path.to_string_lossy().into_owned()]).unwrap();
        let reselected = selected_files_from_paths(&[path.to_string_lossy().into_owned()]).unwrap();
        assert_eq!(
            selected, reselected,
            "same snapshot selection emits cancel, not change"
        );
        let other = directory.0.join("same-bytes.txt");
        fs::copy(&path, &other).unwrap();
        let other = selected_files_from_paths(&[other.to_string_lossy().into_owned()]).unwrap();
        assert_ne!(
            selected[0].data, other[0].data,
            "different disk files retain different identities"
        );
        let data = selected[0].data.clone();
        assert!(
            data.as_bytes().is_none(),
            "selection must not materialize contents"
        );
        assert_eq!(data.identity_hash(), selected[0].data.identity_hash());
        assert_eq!(data.slice(1..4).read_blocking().unwrap(), b"bcd");
        assert_eq!(data.read_blocking().unwrap(), b"abcdef");
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, b"ABCDEF").unwrap();
        fs::File::open(&path)
            .unwrap()
            .set_modified(modified + std::time::Duration::from_secs(2))
            .unwrap();
        assert_eq!(
            data.read_blocking().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::remove_file(&path).unwrap();
        assert_eq!(
            selected[0].data.read_blocking().unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn selected_file_replaced_by_fifo_fails_without_waiting_for_a_writer() {
        use std::{os::unix::fs::FileTypeExt, process::Command, sync::mpsc, time::Duration};

        for directory_selection in [false, true] {
            let scratch = Scratch::new("replaced-fifo");
            let path = scratch.0.join("file.txt");
            fs::write(&path, b"public").unwrap();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            let files = {
                let _entered = runtime.enter();
                let selected = if directory_selection {
                    &scratch.0
                } else {
                    &path
                };
                selected_files_from_paths(&[selected.to_string_lossy().into_owned()]).unwrap()
            };
            fs::remove_file(&path).unwrap();
            assert!(
                Command::new("mkfifo")
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success()
            );
            assert!(fs::metadata(&path).unwrap().file_type().is_fifo());

            let (send, receive) = mpsc::channel();
            files[0].data.read_all(Box::new(move |result| {
                let _ = send.send(result);
            }));
            let result = receive.recv_timeout(Duration::from_secs(5));
            if result.is_err() {
                // Linux permits O_RDWR on a FIFO without a peer. Only after
                // recording failure, unblock a regressed reader for teardown.
                if let Ok(_writer) = fs::OpenOptions::new().read(true).write(true).open(&path) {
                    let _ = receive.recv_timeout(Duration::from_secs(5));
                }
            }
            runtime.shutdown_timeout(Duration::from_secs(1));
            let error = result
                .expect("FIFO read must finish without a writer")
                .unwrap_err();
            assert_eq!(
                moli_file_api::file::read_error_name(&error),
                "NotReadableError"
            );
        }
    }

    #[cfg(all(unix, target_pointer_width = "64"))]
    #[tokio::test]
    async fn sparse_files_larger_than_two_gib_are_selected_without_materializing() {
        let directory = Scratch::new("sparse");
        let path = directory.0.join("large.bin");
        let size = 3 * 1024 * 1024 * 1024usize;
        let mut file = fs::File::create(&path).unwrap();
        file.set_len(size as u64).unwrap();
        file.seek(SeekFrom::End(-4)).unwrap();
        std::io::Write::write_all(&mut file, b"tail").unwrap();
        drop(file);
        let files = selected_files_from_paths(&[path.to_string_lossy().into_owned()]).unwrap();
        assert_eq!(files[0].data.len(), size);
        assert!(files[0].data.as_bytes().is_none());
        assert_eq!(
            files[0].data.slice(size - 4..size).read_blocking().unwrap(),
            b"tail"
        );
    }

    #[test]
    fn a_retired_file_executor_completes_the_read_with_an_error() {
        let directory = Scratch::new("retired");
        let path = directory.0.join("file.txt");
        fs::write(&path, b"data").unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let files = {
            let _entered = runtime.enter();
            selected_files_from_paths(&[path.to_string_lossy().into_owned()]).unwrap()
        };
        drop(runtime);
        assert_eq!(
            files[0].data.read_blocking().unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
    }

    #[test]
    fn cancelled_selection_stops_before_filesystem_admission() {
        let cancelled = std::sync::atomic::AtomicBool::new(true);
        let result = select_files(
            &["missing-file-must-not-be-inspected".into()],
            Some(&cancelled),
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
    }
    #[cfg(unix)]
    fn matching_outside_file(original: &Path, outside: &Path) {
        fs::create_dir_all(outside.parent().unwrap()).unwrap();
        fs::write(outside, b"SECRET").unwrap();
        fs::File::open(outside)
            .unwrap()
            .set_modified(fs::metadata(original).unwrap().modified().unwrap())
            .unwrap();
        assert_eq!(
            fs::metadata(original).unwrap().len(),
            fs::metadata(outside).unwrap().len()
        );
        assert_eq!(
            fs::metadata(original).unwrap().modified().unwrap(),
            fs::metadata(outside).unwrap().modified().unwrap()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn directory_upload_read_rejects_a_replaced_file_symlink_with_matching_metadata() {
        use std::os::unix::fs::symlink;
        let scratch = Scratch::new("replaced-file");
        let root = scratch.0.join("chosen");
        fs::create_dir_all(&root).unwrap();
        let original = root.join("file.txt");
        fs::write(&original, b"public").unwrap();
        let files = selected_files_from_paths(&[root.to_string_lossy().into_owned()]).unwrap();
        let outside = scratch.0.join("outside/secret.txt");
        matching_outside_file(&original, &outside);
        fs::remove_file(&original).unwrap();
        symlink(&outside, &original).unwrap();
        assert_eq!(
            files[0].data.read_blocking().unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(
            files[0]
                .data
                .slice(1..3)
                .read_blocking()
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn directory_upload_read_rejects_replaced_parent_and_root_symlinks() {
        use std::os::unix::fs::symlink;
        for (replaced, tail) in [
            ("selected-parent", "chosen/nested/deep/file.txt"),
            ("selected-parent/chosen", "nested/deep/file.txt"),
            ("selected-parent/chosen/nested", "deep/file.txt"),
            ("selected-parent/chosen/nested/deep", "file.txt"),
        ] {
            let scratch = Scratch::new("replaced-parent");
            let root = scratch.0.join("selected-parent/chosen");
            let original = root.join("nested/deep/file.txt");
            fs::create_dir_all(original.parent().unwrap()).unwrap();
            fs::write(&original, b"public").unwrap();
            let files = selected_files_from_paths(&[root.to_string_lossy().into_owned()]).unwrap();
            let outside = scratch.0.join("outside");
            matching_outside_file(&original, &outside.join(tail));
            let replaced = scratch.0.join(replaced);
            fs::rename(&replaced, scratch.0.join("retained-directory")).unwrap();
            symlink(&outside, &replaced).unwrap();
            assert_eq!(
                files[0].data.read_blocking().unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn directory_upload_keeps_the_resolved_explicit_root_when_its_link_is_retargeted() {
        use std::os::unix::fs::symlink;
        let scratch = Scratch::new("retargeted-root-link");
        let root = scratch.0.join("real-root");
        fs::create_dir_all(&root).unwrap();
        let original = root.join("file.txt");
        fs::write(&original, b"public").unwrap();
        let selected_link = scratch.0.join("chosen-link");
        symlink(&root, &selected_link).unwrap();
        let files =
            selected_files_from_paths(&[selected_link.to_string_lossy().into_owned()]).unwrap();
        let outside = scratch.0.join("outside");
        matching_outside_file(&original, &outside.join("file.txt"));
        fs::remove_file(&selected_link).unwrap();
        symlink(&outside, &selected_link).unwrap();
        assert_eq!(files[0].relative_path, "chosen-link/file.txt");
        assert_eq!(files[0].data.read_blocking().unwrap(), b"public");
    }

    #[cfg(unix)]
    #[test]
    fn directory_upload_rechecks_a_queued_directory_before_enumerating() {
        use std::os::unix::fs::symlink;
        let scratch = Scratch::new("queued-directory");
        let root = scratch.0.join("chosen");
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(&root).unwrap();
        let queued = root.join("queued");
        fs::create_dir_all(&queued).unwrap();
        // Record the same directory kind used by the enumeration queue.
        let entry = fs::read_dir(&root).unwrap().next().unwrap().unwrap();
        assert!(entry.file_type().unwrap().is_dir());
        let outside = scratch.0.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), b"SECRET").unwrap();
        fs::remove_dir(&queued).unwrap();
        symlink(&outside, &queued).unwrap();
        assert_eq!(
            read_upload_directory(&root, &entry.path())
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[cfg(unix)]
    #[test]
    fn directory_upload_rechecks_a_file_before_creating_its_backing() {
        use std::os::unix::fs::symlink;
        let scratch = Scratch::new("queued-file");
        let root = scratch.0.join("chosen");
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(&root).unwrap();
        let file = root.join("file.txt");
        fs::write(&file, b"public").unwrap();
        let entry = fs::read_dir(&root).unwrap().next().unwrap().unwrap();
        assert!(entry.file_type().unwrap().is_file());
        let outside = scratch.0.join("outside.txt");
        matching_outside_file(&file, &outside);
        fs::remove_file(&file).unwrap();
        symlink(&outside, &file).unwrap();
        let root: Arc<Path> = root.into();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        assert_eq!(
            selected_file_from_file_path(
                &entry.path(),
                runtime.handle(),
                FileReadPolicy::DirectoryMember { root },
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn directory_upload_paths_cannot_traverse_out_of_the_recorded_root() {
        let scratch = Scratch::new("path-boundary");
        let root = fs::canonicalize(&scratch.0).unwrap();
        fs::write(root.join("file.txt"), b"public").unwrap();
        for path in [
            root.parent().unwrap().join("outside.txt"),
            root.join("../outside.txt"),
        ] {
            assert_eq!(
                directory_upload_metadata(&root, &path).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn explicitly_selected_file_links_keep_their_existing_read_behavior() {
        use std::os::unix::fs::symlink;
        let scratch = Scratch::new("explicit-file-link");
        let file = scratch.0.join("original.txt");
        fs::write(&file, b"public").unwrap();
        let link = scratch.0.join("chosen-link.txt");
        symlink(&file, &link).unwrap();
        let files = selected_files_from_paths(&[link.to_string_lossy().into_owned()]).unwrap();
        assert_eq!(files[0].name, "chosen-link.txt");
        assert!(files[0].relative_path.is_empty());
        assert_eq!(files[0].data.read_blocking().unwrap(), b"public");
    }
}
