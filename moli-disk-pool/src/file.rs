use std::{fs::File, io};

#[cfg(not(any(unix, windows)))]
use parking_lot::Mutex;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) struct PoolFile {
    #[cfg(any(unix, windows))]
    file: File,
    #[cfg(not(any(unix, windows)))]
    file: Mutex<File>,
    #[cfg(any(test, feature = "test-support"))]
    fail_next_write: AtomicBool,
}

impl PoolFile {
    pub(crate) fn new(file: File) -> Self {
        Self {
            #[cfg(any(unix, windows))]
            file,
            #[cfg(not(any(unix, windows)))]
            file: Mutex::new(file),
            #[cfg(any(test, feature = "test-support"))]
            fail_next_write: AtomicBool::new(false),
        }
    }

    pub(crate) fn read_exact_at(&self, mut buffer: &mut [u8], mut offset: u64) -> io::Result<()> {
        while !buffer.is_empty() {
            match self.read_at(buffer, offset) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "disk pool file ended inside an allocated extent",
                    ));
                }
                Ok(read) => {
                    offset = offset
                        .checked_add(u64::try_from(read).map_err(|_| {
                            io::Error::new(io::ErrorKind::InvalidInput, "read size is too large")
                        })?)
                        .ok_or_else(|| {
                            io::Error::new(io::ErrorKind::InvalidInput, "read offset overflow")
                        })?;
                    buffer = &mut buffer[read..];
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    pub(crate) fn write_all_at(&self, mut data: &[u8], mut offset: u64) -> io::Result<()> {
        #[cfg(any(test, feature = "test-support"))]
        if self.fail_next_write.swap(false, Ordering::SeqCst) {
            return Err(io::Error::other("injected disk pool write failure"));
        }

        while !data.is_empty() {
            match self.write_at(data, offset) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "failed to write an allocated disk extent",
                    ));
                }
                Ok(written) => {
                    offset = offset
                        .checked_add(u64::try_from(written).map_err(|_| {
                            io::Error::new(io::ErrorKind::InvalidInput, "write size is too large")
                        })?)
                        .ok_or_else(|| {
                            io::Error::new(io::ErrorKind::InvalidInput, "write offset overflow")
                        })?;
                    data = &data[written..];
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn fail_next_write_for_test(&self) {
        self.fail_next_write.store(true, Ordering::SeqCst);
    }

    #[cfg(unix)]
    fn read_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        use std::os::unix::fs::FileExt;

        self.file.read_at(buffer, offset)
    }

    #[cfg(unix)]
    fn write_at(&self, data: &[u8], offset: u64) -> io::Result<usize> {
        use std::os::unix::fs::FileExt;

        self.file.write_at(data, offset)
    }

    #[cfg(windows)]
    fn read_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        use std::os::windows::fs::FileExt;

        self.file.seek_read(buffer, offset)
    }

    #[cfg(windows)]
    fn write_at(&self, data: &[u8], offset: u64) -> io::Result<usize> {
        use std::os::windows::fs::FileExt;

        self.file.seek_write(data, offset)
    }

    #[cfg(not(any(unix, windows)))]
    fn read_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        use std::io::{Read, Seek};

        let mut file = self.file.lock();
        file.seek(io::SeekFrom::Start(offset))?;
        file.read(buffer)
    }

    #[cfg(not(any(unix, windows)))]
    fn write_at(&self, data: &[u8], offset: u64) -> io::Result<usize> {
        use std::io::{Seek, Write};

        let mut file = self.file.lock();
        file.seek(io::SeekFrom::Start(offset))?;
        file.write(data)
    }

    #[cfg(feature = "test-support")]
    pub(crate) fn set_len(&self, len: u64) -> io::Result<()> {
        #[cfg(any(unix, windows))]
        {
            self.file.set_len(len)
        }
        #[cfg(not(any(unix, windows)))]
        {
            self.file.lock().set_len(len)
        }
    }
}
