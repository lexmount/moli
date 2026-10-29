mod parse;
mod serialize;
mod types;

pub use parse::parse_multipart_form_data;
pub use serialize::{
    serialize_multipart_form_data_with_encoding, serialize_multipart_form_data_with_prefix,
};
pub use types::{
    DEFAULT_MULTIPART_BLOB_CONTENT_TYPE, DEFAULT_MULTIPART_PARSED_FILE_CONTENT_TYPE,
    MultipartFormDataEntry, MultipartFormDataPart, MultipartFormDataPartValue,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialize::{
        escape_multipart_name, multipart_boundary_collides, multipart_boundary_with_candidate,
        serialize_multipart_form_data_body,
    };

    fn text_part(name: &str, value: &str) -> MultipartFormDataPart {
        MultipartFormDataPart {
            name: name.to_owned(),
            value: MultipartFormDataPartValue::Text(value.to_owned()),
        }
    }

    fn blob_part(
        name: &str,
        filename: &str,
        content_type: &str,
        body: &[u8],
    ) -> MultipartFormDataPart {
        MultipartFormDataPart {
            name: name.to_owned(),
            value: MultipartFormDataPartValue::Blob {
                filename: filename.to_owned(),
                content_type: content_type.to_owned(),
                body: body.to_vec(),
            },
        }
    }

    #[test]
    fn escape_multipart_name_uses_percent_encoding_for_header_parameters() {
        assert_eq!(
            escape_multipart_name(b"line\nbreak\r\"quote\""),
            b"line%0Abreak%0D%22quote%22"
        );
        assert_eq!(
            escape_multipart_name("cafe-\u{e9}".as_bytes()),
            "cafe-\u{e9}".as_bytes()
        );
    }

    #[test]
    fn multipart_boundary_selection_skips_colliding_candidate() {
        let parts = [text_part("field", "before--forced-boundary--after")];

        let selected =
            multipart_boundary_with_candidate(&parts, encoding_rs::UTF_8, |salt| match salt {
                0 => "forced-boundary".to_owned(),
                1 => "safe-boundary".to_owned(),
                _ => panic!("selection should stop after the first non-colliding candidate"),
            });

        assert_eq!(selected, "safe-boundary");
    }

    #[test]
    fn multipart_boundary_collision_scans_names_text_and_blob_metadata() {
        let parts = [
            text_part("name-collision", "plain"),
            text_part("field", "value-collision"),
            blob_part("blob", "filename-collision.txt", "text/plain", b"body"),
            blob_part("typed", "file.txt", "application/collision", b"body"),
        ];

        assert!(multipart_boundary_collides(
            &parts,
            "name-collision",
            encoding_rs::UTF_8
        ));
        assert!(multipart_boundary_collides(
            &parts,
            "value-collision",
            encoding_rs::UTF_8
        ));
        assert!(multipart_boundary_collides(
            &parts,
            "filename-collision",
            encoding_rs::UTF_8
        ));
        assert!(multipart_boundary_collides(
            &parts,
            "application/collision",
            encoding_rs::UTF_8
        ));
        assert!(!multipart_boundary_collides(
            &parts,
            "missing-boundary",
            encoding_rs::UTF_8
        ));
    }

    #[test]
    fn multipart_boundary_collision_scans_blob_bytes() {
        let parts = [blob_part(
            "file",
            "payload.bin",
            "application/octet-stream",
            b"\x00prefix--binary-boundary--suffix\xff",
        )];

        assert!(multipart_boundary_collides(
            &parts,
            "binary-boundary",
            encoding_rs::UTF_8
        ));
        assert!(!multipart_boundary_collides(
            &parts,
            "text-boundary",
            encoding_rs::UTF_8
        ));
    }

    #[test]
    fn multipart_blob_parts_default_empty_mime_type_to_octet_stream() {
        let parts = [blob_part("upload", "empty-type.bin", "", b"bytes")];

        let body = String::from_utf8(serialize_multipart_form_data_body(
            &parts,
            "boundary",
            encoding_rs::UTF_8,
        ))
        .unwrap();

        assert_eq!(
            body,
            "--boundary\r\n\
             Content-Disposition: form-data; name=\"upload\"; filename=\"empty-type.bin\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n\
             bytes\r\n\
             --boundary--\r\n"
        );
    }

    #[test]
    fn multipart_serialization_normalizes_text_without_changing_filenames_or_file_bytes() {
        let parts = [
            text_part("line\nbreak\r\n\"", "a\rb\nc\r\nd\n\r星\u{2028}"),
            blob_part("file\rfield", "raw\n\r\".bin", "", b"raw\r\n\0\xff"),
        ];
        let original = parts.clone();
        let body = serialize_multipart_form_data_body(&parts, "boundary", encoding_rs::UTF_8);
        let mut expected = concat!(
            "--boundary\r\n",
            "Content-Disposition: form-data; name=\"line%0D%0Abreak%0D%0A%22\"\r\n\r\n",
            "a\r\nb\r\nc\r\nd\r\n\r\n星\u{2028}\r\n",
            "--boundary\r\n",
            "Content-Disposition: form-data; name=\"file%0D%0Afield\"; filename=\"raw%0A%0D%22.bin\"\r\n",
            "Content-Type: application/octet-stream\r\n\r\n",
        ).as_bytes().to_vec();
        expected.extend_from_slice(b"raw\r\n\0\xff\r\n--boundary--\r\n");
        assert_eq!(body, expected);
        assert_eq!(parts, original);
    }

    #[test]
    fn multipart_legacy_encoding_uses_numeric_references_and_preserves_binary_data() {
        let parts = [
            text_part("aəb\n", "c\u{fffd}d\r"),
            blob_part("á", "💩\n.bin", "", b"\xff\r\n"),
        ];
        let body =
            serialize_multipart_form_data_body(&parts, "boundary", encoding_rs::WINDOWS_1252);
        let mut expected = concat!(
            "--boundary\r\n",
            "Content-Disposition: form-data; name=\"a&#601;b%0D%0A\"\r\n\r\n",
            "c&#65533;d\r\n\r\n",
            "--boundary\r\n",
            "Content-Disposition: form-data; name=\"",
        )
        .as_bytes()
        .to_vec();
        expected.push(0xe1);
        expected.extend_from_slice(
            concat!(
                "\"; filename=\"&#128169;%0A.bin\"\r\n",
                "Content-Type: application/octet-stream\r\n\r\n",
            )
            .as_bytes(),
        );
        expected.extend_from_slice(b"\xff\r\n\r\n--boundary--\r\n");
        assert_eq!(body, expected);

        let (body, content_type) = serialize_multipart_form_data_with_encoding(
            &parts,
            "multipart-",
            encoding_rs::WINDOWS_1252,
        );
        let boundary = content_type
            .strip_prefix("multipart/form-data; boundary=")
            .unwrap();
        let parsed = parse_multipart_form_data(&body, boundary).unwrap();
        assert_eq!(parsed[0].name, "a&#601;b%0D%0A");
        assert_eq!(parsed[0].body, b"c&#65533;d\r\n");
        assert_eq!(parsed[1].body, b"\xff\r\n");
    }

    #[test]
    fn multipart_boundary_selection_checks_encoded_text_and_escaped_parameters() {
        let parts = [text_part("field", "一")];
        assert!(multipart_boundary_collides(
            &parts,
            "0l",
            encoding_rs::ISO_2022_JP
        ));
        assert!(!multipart_boundary_collides(
            &parts,
            "0l",
            encoding_rs::UTF_8
        ));
        let boundary =
            multipart_boundary_with_candidate(&parts, encoding_rs::ISO_2022_JP, |salt| {
                if salt == 0 {
                    "0l".to_owned()
                } else {
                    "safe-boundary".to_owned()
                }
            });
        assert_eq!(boundary, "safe-boundary");
        assert!(multipart_boundary_collides(
            &[text_part("\n", "plain")],
            "0D",
            encoding_rs::UTF_8
        ));
        assert!(multipart_boundary_collides(
            &[blob_part("file", "\n", "", b"plain")],
            "0A",
            encoding_rs::UTF_8
        ));
    }

    #[test]
    fn parses_multipart_text_and_file_entries() {
        let body = b"--boundary\r\n\
Content-Disposition: form-data; name=\"field\"\r\n\
\r\n\
value\r\n\
--boundary\r\n\
content-disposition: form-data; name=\"upload\"; filename=\"file.json\"\r\n\
content-type: Application/JSON\r\n\
\r\n\
{\"ok\":true}\r\n\
--boundary--\r\n";

        let entries = parse_multipart_form_data(body, "boundary").unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "field");
        assert_eq!(entries[0].filename, None);
        assert_eq!(entries[0].body, b"value");
        assert_eq!(entries[1].name, "upload");
        assert_eq!(entries[1].filename.as_deref(), Some("file.json"));
        assert_eq!(entries[1].content_type, "application/json");
        assert_eq!(entries[1].body, br#"{"ok":true}"#);
    }

    #[test]
    fn parsed_file_parts_without_content_type_default_to_text_plain() {
        let body = b"--boundary\r\n\
Content-Disposition: form-data; name=\"field\"\r\n\
\r\n\
value\r\n\
--boundary\r\n\
Content-Disposition: form-data; name=\"upload\"; filename=\"file.txt\"\r\n\
\r\n\
file-body\r\n\
--boundary--\r\n";

        let entries = parse_multipart_form_data(body, "boundary").unwrap();

        assert_eq!(entries[0].filename, None);
        assert_eq!(entries[0].content_type, "");
        assert_eq!(entries[1].filename.as_deref(), Some("file.txt"));
        assert_eq!(
            entries[1].content_type,
            DEFAULT_MULTIPART_PARSED_FILE_CONTENT_TYPE
        );
    }

    #[test]
    fn parses_capitalized_boundary_values_case_sensitively() {
        let body = b"--Boundary_with_capital_letters\r\n\
Content-Type: application/json\r\n\
Content-Disposition: form-data; name=\"does_this_work\"\r\n\
\r\n\
YES\r\n\
--Boundary_with_capital_letters--\r\n";

        let entries = parse_multipart_form_data(body, "Boundary_with_capital_letters").unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "does_this_work");
        assert_eq!(entries[0].body, b"YES");
    }

    #[test]
    fn parses_transport_padding_after_boundary_delimiters() {
        let body = b"--boundary \t\r\n\
Content-Disposition: form-data; name=\"first\"\r\n\
\r\n\
one\r\n\
--boundary\t \r\n\
Content-Disposition: form-data; name=\"second\"\r\n\
\r\n\
two\r\n\
--boundary-- \t\r\n";

        let entries = parse_multipart_form_data(body, "boundary").unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "first");
        assert_eq!(entries[0].body, b"one");
        assert_eq!(entries[1].name, "second");
        assert_eq!(entries[1].body, b"two");
    }

    #[test]
    fn rejects_malformed_boundary_suffixes() {
        let body = b"--Boundary_with_capital_letters\r\n\
Content-Type: application/json\r\n\
Content-Disposition: form-data; name=\"does_this_work\"\r\n\
\r\n\
YES\r\n\
--Boundary_with_capital_letters-Random junk";

        assert!(parse_multipart_form_data(body, "Boundary_with_capital_letters").is_none());
    }

    #[test]
    fn parses_empty_multipart_body() {
        for body in [b"--boundary--".as_slice(), b"--boundary--\r\n"] {
            let entries = parse_multipart_form_data(body, "boundary").unwrap();
            assert!(entries.is_empty());
        }
    }

    #[test]
    fn preserves_percent_escaped_names_and_filenames_like_chromium() {
        let body = b"--boundary\r\n\
Content-Disposition: form-data; name=\"line%0Dbreak\"; filename=\"a%22b.txt\"\r\n\
\r\n\
body\r\n\
--boundary--\r\n";

        let entries = parse_multipart_form_data(body, "boundary").unwrap();

        assert_eq!(entries[0].name, "line%0Dbreak");
        assert_eq!(entries[0].filename.as_deref(), Some("a%22b.txt"));
    }

    #[test]
    fn rejects_boundary_prefixes_at_the_start_of_part_body_lines() {
        let body = b"--boundary\r\n\
Content-Disposition: form-data; name=\"field\"\r\n\
\r\n\
before\r\n--boundary-like text\r\n\
after\r\n\
--boundary--\r\n";

        assert!(parse_multipart_form_data(body, "boundary").is_none());
    }
}
