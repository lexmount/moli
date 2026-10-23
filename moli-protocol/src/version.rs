//! Shared CDP version metadata.

pub const MOLI_VERSION: &str = env!("MOLI_VERSION");
pub const PROTOCOL_VERSION: &str = "1.3";
pub const PRODUCT: &str = moli_browser_profile::DEFAULT_CDP_PRODUCT;
pub const REVISION: &str = concat!("@", env!("VERGEN_GIT_SHA"));
pub const WEBKIT_VERSION: &str = "537.36";

pub fn js_version() -> &'static str {
    v8::V8::get_version()
}

#[cfg(test)]
mod tests {
    use super::REVISION;
    use std::process::Command;

    #[test]
    fn revision_is_the_full_build_source_commit() {
        let hash = REVISION
            .strip_prefix('@')
            .expect("CDP revision must use Chromium's @<commit> shape");
        assert!(matches!(hash.len(), 40 | 64), "revision={REVISION}");
        assert!(
            hash.chars().all(|character| character.is_ascii_hexdigit()),
            "revision={REVISION}"
        );

        let output = Command::new("git")
            .args([
                "-C",
                concat!(env!("CARGO_MANIFEST_DIR"), "/.."),
                "rev-parse",
                "--verify",
                "HEAD",
            ])
            .output()
            .expect("run git rev-parse for build revision regression");
        assert!(output.status.success(), "git rev-parse failed: {output:?}");
        let checkout_revision =
            String::from_utf8(output.stdout).expect("git revision must be UTF-8");
        assert_eq!(hash, checkout_revision.trim());
    }
}
