use crate::resource_mime_essence_for_path;

/// Determine the type of a file selected from disk from its filename, as
/// Chromium does. Unknown extensions yield an empty type; contents are not
/// sniffed. The multipart encoder supplies its own fallback when needed.
pub fn selected_file_mime_type(name: &str) -> &'static str {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "apng" => Some("image/apng"),
        "avif" => Some("image/avif"),
        "webp" => Some("image/webp"),
        "ico" => Some("image/x-icon"),
        "tif" | "tiff" => Some("image/tiff"),
        "jpe" | "jfif" => Some("image/jpeg"),
        "svgz" => Some("image/svg+xml"),
        "json" => Some("application/json"),
        "csv" => Some("text/csv"),
        "js" | "mjs" => Some("text/javascript"),
        "xml" | "xsl" | "xslt" => Some("text/xml"),
        "text" => Some("text/plain"),
        "shtml" => Some("text/html"),
        "pdf" => Some("application/pdf"),
        "zip" => Some("application/zip"),
        "gz" | "tgz" => Some("application/gzip"),
        "tar" => Some("application/x-tar"),
        "7z" => Some("application/x-7z-compressed"),
        "rar" => Some("application/vnd.rar"),
        "doc" | "dot" => Some("application/msword"),
        "docx" => Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        "xls" => Some("application/vnd.ms-excel"),
        "xlsx" => Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        "ppt" => Some("application/vnd.ms-powerpoint"),
        "pptx" => Some("application/vnd.openxmlformats-officedocument.presentationml.presentation"),
        "rtf" => Some("application/rtf"),
        "epub" => Some("application/epub+zip"),
        "wasm" => Some("application/wasm"),
        "bin" | "exe" | "com" => Some("application/octet-stream"),
        "wav" => Some("audio/wav"),
        "ogg" | "oga" | "opus" => Some("audio/ogg"),
        "flac" => Some("audio/flac"),
        "m4a" => Some("audio/x-m4a"),
        "m4v" => Some("video/mp4"),
        "ogv" => Some("video/ogg"),
        "vtt" => Some("text/vtt"),
        _ => resource_mime_essence_for_path(name),
    }
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::selected_file_mime_type;

    #[test]
    fn selected_file_types_use_case_insensitive_extensions() {
        for (name, expected) in [
            ("note.TXT", "text/plain"),
            ("rows.csv", "text/csv"),
            ("data.json", "application/json"),
            ("document.pdf", "application/pdf"),
            ("image.PNG", "image/png"),
            ("empty.jpeg", "image/jpeg"),
            (
                "document.docx",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            ),
            ("audio.wav", "audio/wav"),
            ("document.xml", "text/xml"),
            ("binary.bin", "application/octet-stream"),
            (".json", "application/json"),
            ("archive.tar.GZ", "application/gzip"),
        ] {
            assert_eq!(selected_file_mime_type(name), expected, "{name}");
        }
    }

    #[test]
    fn selected_file_types_leave_unknown_extensions_and_extensionless_names_empty() {
        for name in [
            "image",
            "document.unknown",
            "archive",
            "font",
            "empty",
            "file.",
            "pdf",
            ".unknown",
        ] {
            assert_eq!(selected_file_mime_type(name), "", "{name}");
        }
    }
}
