use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use flate2::read::DeflateDecoder;
use moli_core::page::SelectedFile;
use moli_protocol::automation::{DevToolsError, DevToolsErrorKind};

const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;
const MAX_UPLOAD_ZIP_ARCHIVE_BYTES: usize = MAX_UPLOAD_BYTES + 1024 * 1024;
const MAX_UPLOAD_BASE64_CHARS: usize = MAX_UPLOAD_ZIP_ARCHIVE_BYTES.div_ceil(3) * 4;
const MAX_DOWNLOAD_BYTES: usize = 50 * 1024 * 1024;

pub(super) fn selected_files_from_paths(
    paths: &[String],
    label: &str,
) -> Result<Vec<SelectedFile>, DevToolsError> {
    paths
        .iter()
        .map(|path| selected_file_from_path(path, label))
        .collect()
}

fn selected_file_from_path(path: &str, label: &str) -> Result<SelectedFile, DevToolsError> {
    let file_path = Path::new(path);
    let metadata = fs::metadata(file_path).map_err(|_| {
        DevToolsError::new(
            DevToolsErrorKind::UnableToSetFileInput,
            format!("File not found : {path}"),
        )
    })?;
    if !metadata.is_file() {
        return Err(DevToolsError::new(
            DevToolsErrorKind::UnableToSetFileInput,
            format!("File not found : {path}"),
        ));
    }
    let bytes = fs::read(file_path).map_err(|error| {
        DevToolsError::new(
            DevToolsErrorKind::UnableToSetFileInput,
            format!("could not read file for {label}: {error}"),
        )
    })?;
    let name = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_owned();
    let last_modified = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis() as f64)
                .unwrap_or(0.0)
        });
    Ok(SelectedFile {
        bytes,
        mime_type: String::new(),
        name,
        last_modified,
    })
}

pub(super) fn uploaded_file_from_base64_zip(
    base64_zip: &str,
    session_id: &str,
) -> Result<std::path::PathBuf, String> {
    let compact = compact_base64_upload_payload(base64_zip, MAX_UPLOAD_BASE64_CHARS)?;
    let zip_data = BASE64_STANDARD
        .decode(compact)
        .map_err(|error| format!("unable to decode 'file': {error}"))?;
    if zip_data.len() > MAX_UPLOAD_ZIP_ARCHIVE_BYTES {
        return Err(format!("ZIP archive too large: {} bytes", zip_data.len()));
    }
    let entry = sole_zip_file_entry(&zip_data)?;
    let directory = unique_upload_directory(session_id);
    fs::create_dir_all(&directory)
        .map_err(|error| format!("unable to create upload directory: {error}"))?;
    let path = directory.join(entry.name);
    if let Err(error) = fs::write(&path, entry.bytes) {
        let _ = fs::remove_dir(&directory);
        return Err(format!("unable to write uploaded file: {error}"));
    }
    Ok(path)
}

pub(super) fn downloadable_file_zip_base64(
    file_name: &str,
    bytes: &[u8],
) -> Result<String, String> {
    if bytes.len() > MAX_DOWNLOAD_BYTES {
        return Err(format!("download file too large: {} bytes", bytes.len()));
    }
    let name = safe_upload_file_name(file_name)?;
    let zip = stored_zip_single_file(&name, bytes)?;
    Ok(BASE64_STANDARD.encode(zip))
}

pub(super) fn downloadable_file_bytes(file_path: &Path) -> Result<Vec<u8>, String> {
    read_file_bytes_with_limit(file_path, MAX_DOWNLOAD_BYTES)
}

pub(super) fn unique_download_directory(session_id: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "moli-webdriver-download-{}-{}-{nonce}",
        std::process::id(),
        sanitize_path_segment(session_id)
    ))
}

fn read_file_bytes_with_limit(file_path: &Path, max_bytes: usize) -> Result<Vec<u8>, String> {
    let file = fs::File::open(file_path)
        .map_err(|error| format!("unable to read download file: {error}"))?;
    let mut reader = file.take(max_bytes.saturating_add(1) as u64);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| format!("unable to read download file: {error}"))?;
    if bytes.len() > max_bytes {
        return Err(format!(
            "download file too large: more than {max_bytes} bytes"
        ));
    }
    Ok(bytes)
}

fn compact_base64_upload_payload(base64_zip: &str, max_chars: usize) -> Result<String, String> {
    let mut compact = String::new();
    for character in base64_zip.chars() {
        if character.is_ascii_whitespace() {
            continue;
        }
        if compact.len() >= max_chars {
            return Err(format!(
                "ZIP upload payload too large: more than {max_chars} base64 chars"
            ));
        }
        compact.push(character);
    }
    Ok(compact)
}

struct UploadedZipEntry {
    name: String,
    bytes: Vec<u8>,
}

fn sole_zip_file_entry(zip_data: &[u8]) -> Result<UploadedZipEntry, String> {
    if zip_data.len() < 30 || &zip_data[0..4] != b"PK\x03\x04" {
        return Err("upload payload is not a ZIP local file entry".to_owned());
    }
    let flags = read_u16_le(zip_data, 6)?;
    if flags & 0x0008 != 0 {
        return Err("ZIP data descriptors are not supported for WebDriver file upload".to_owned());
    }
    let compression = read_u16_le(zip_data, 8)?;
    let compressed_size = read_u32_le(zip_data, 18)? as usize;
    let uncompressed_size = read_u32_le(zip_data, 22)? as usize;
    if uncompressed_size > MAX_UPLOAD_BYTES {
        return Err(format!("ZIP entry too large: {uncompressed_size} bytes"));
    }
    let name_len = read_u16_le(zip_data, 26)? as usize;
    let extra_len = read_u16_le(zip_data, 28)? as usize;
    let name_start = 30;
    let name_end = checked_add(name_start, name_len)?;
    let data_start = checked_add(name_end, extra_len)?;
    let data_end = checked_add(data_start, compressed_size)?;
    if data_end > zip_data.len() {
        return Err("ZIP entry extends beyond upload payload".to_owned());
    }
    if zip_data[data_end..]
        .windows(4)
        .any(|signature| signature == b"PK\x03\x04")
    {
        return Err("WebDriver file upload ZIP must contain exactly one file".to_owned());
    }
    let entry_name = std::str::from_utf8(&zip_data[name_start..name_end])
        .map_err(|error| format!("ZIP entry name is not UTF-8: {error}"))?;
    let name = safe_upload_file_name(entry_name)?;
    let compressed = &zip_data[data_start..data_end];
    let bytes = match compression {
        0 => compressed.to_vec(),
        8 => {
            let mut decoder = DeflateDecoder::new(compressed);
            let mut out = Vec::with_capacity(uncompressed_size);
            decoder
                .read_to_end(&mut out)
                .map_err(|error| format!("unable to inflate ZIP entry: {error}"))?;
            out
        }
        other => {
            return Err(format!("unsupported ZIP compression method {other}"));
        }
    };
    if bytes.len() != uncompressed_size {
        return Err(format!(
            "ZIP entry size mismatch: expected {uncompressed_size}, got {}",
            bytes.len()
        ));
    }
    Ok(UploadedZipEntry { name, bytes })
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let data = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| "truncated ZIP header".to_owned())?;
    Ok(u16::from_le_bytes([data[0], data[1]]))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let data = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| "truncated ZIP header".to_owned())?;
    Ok(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
}

fn checked_add(left: usize, right: usize) -> Result<usize, String> {
    left.checked_add(right)
        .ok_or_else(|| "ZIP entry offset overflow".to_owned())
}

fn stored_zip_single_file(file_name: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let name = file_name.as_bytes();
    let name_len = u16::try_from(name.len()).map_err(|_| "ZIP entry file name is too long")?;
    let body_len = u32::try_from(bytes.len()).map_err(|_| "ZIP entry is too large")?;
    let crc = crc32(bytes);
    let mut zip = Vec::with_capacity(30 + name.len() + bytes.len() + 46 + name.len() + 22);

    zip.extend_from_slice(b"PK\x03\x04");
    zip.extend_from_slice(&20_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&crc.to_le_bytes());
    zip.extend_from_slice(&body_len.to_le_bytes());
    zip.extend_from_slice(&body_len.to_le_bytes());
    zip.extend_from_slice(&name_len.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(name);
    zip.extend_from_slice(bytes);

    let central_directory_offset =
        u32::try_from(zip.len()).map_err(|_| "ZIP central directory offset overflow")?;
    zip.extend_from_slice(b"PK\x01\x02");
    zip.extend_from_slice(&20_u16.to_le_bytes());
    zip.extend_from_slice(&20_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&crc.to_le_bytes());
    zip.extend_from_slice(&body_len.to_le_bytes());
    zip.extend_from_slice(&body_len.to_le_bytes());
    zip.extend_from_slice(&name_len.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u32.to_le_bytes());
    zip.extend_from_slice(&0_u32.to_le_bytes());
    zip.extend_from_slice(name);

    let central_directory_size = u32::try_from(zip.len())
        .ok()
        .and_then(|end| end.checked_sub(central_directory_offset))
        .ok_or_else(|| "ZIP central directory size overflow".to_owned())?;
    zip.extend_from_slice(b"PK\x05\x06");
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&1_u16.to_le_bytes());
    zip.extend_from_slice(&1_u16.to_le_bytes());
    zip.extend_from_slice(&central_directory_size.to_le_bytes());
    zip.extend_from_slice(&central_directory_offset.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());

    Ok(zip)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn safe_upload_file_name(name: &str) -> Result<String, String> {
    let candidate = name
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .ok_or_else(|| "ZIP entry file name is empty".to_owned())?;
    Ok(candidate.to_owned())
}

fn unique_upload_directory(session_id: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "moli-webdriver-upload-{}-{}-{nonce}",
        std::process::id(),
        sanitize_path_segment(session_id)
    ))
}

fn sanitize_path_segment(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => character,
            _ => '_',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_local_file_entry(
        compression: u16,
        compressed_size: u32,
        uncompressed_size: u32,
        name: &[u8],
        body: &[u8],
    ) -> Vec<u8> {
        let mut zip = Vec::new();
        zip.extend_from_slice(b"PK\x03\x04");
        zip.extend_from_slice(&20_u16.to_le_bytes());
        zip.extend_from_slice(&0_u16.to_le_bytes());
        zip.extend_from_slice(&compression.to_le_bytes());
        zip.extend_from_slice(&0_u16.to_le_bytes());
        zip.extend_from_slice(&0_u16.to_le_bytes());
        zip.extend_from_slice(&0_u32.to_le_bytes());
        zip.extend_from_slice(&compressed_size.to_le_bytes());
        zip.extend_from_slice(&uncompressed_size.to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0_u16.to_le_bytes());
        zip.extend_from_slice(name);
        zip.extend_from_slice(body);
        zip
    }

    #[test]
    fn webdriver_upload_zip_rejects_large_uncompressed_size_before_inflate() {
        let zip = zip_local_file_entry(
            8,
            0,
            u32::try_from(MAX_UPLOAD_BYTES + 1).expect("max upload cap fits u32"),
            b"huge.txt",
            b"",
        );

        let error = match sole_zip_file_entry(&zip) {
            Ok(_) => panic!("oversized upload should fail"),
            Err(error) => error,
        };

        assert_eq!(
            error,
            format!("ZIP entry too large: {} bytes", MAX_UPLOAD_BYTES + 1)
        );
    }

    #[test]
    fn webdriver_upload_zip_rejects_large_base64_before_decode() {
        let error = match compact_base64_upload_payload("AAAA A", 4) {
            Ok(_) => panic!("oversized base64 upload should fail"),
            Err(error) => error,
        };

        assert_eq!(
            error,
            "ZIP upload payload too large: more than 4 base64 chars"
        );
    }

    #[test]
    fn webdriver_download_zip_base64_round_trips_with_upload_parser() {
        let encoded = downloadable_file_zip_base64("../report.txt", b"Hello, World!")
            .expect("download ZIP should encode");
        let zip = BASE64_STANDARD
            .decode(encoded)
            .expect("download ZIP should be valid base64");
        let entry = sole_zip_file_entry(&zip).expect("download ZIP should parse as one entry");

        assert_eq!(entry.name, "report.txt");
        assert_eq!(entry.bytes, b"Hello, World!");
    }

    #[test]
    fn webdriver_download_file_reader_rejects_after_limit_without_full_read() {
        let directory = unique_download_directory("download-reader-limit-test");
        fs::create_dir_all(&directory).expect("test download directory should be created");
        let path = directory.join("large.txt");
        fs::write(&path, b"abcdef").expect("test download file should be written");

        let error = match read_file_bytes_with_limit(&path, 4) {
            Ok(_) => panic!("oversized download file should fail"),
            Err(error) => error,
        };

        assert_eq!(error, "download file too large: more than 4 bytes");
        let _ = fs::remove_dir_all(&directory);
    }
}
