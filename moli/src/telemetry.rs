use tracing_subscriber::{EnvFilter, filter::LevelFilter};

const DEFAULT_LOG_LEVEL: LevelFilter = LevelFilter::INFO;
const FIRST_PARTY_TARGET_PREFIX: &str = "moli";

fn first_party_filter(log_level: &str) -> EnvFilter {
    let level = log_level
        .parse::<LevelFilter>()
        .unwrap_or(DEFAULT_LOG_LEVEL);
    EnvFilter::new(format!("off,{FIRST_PARTY_TARGET_PREFIX}={level}"))
}

pub fn init(log_filter: &str) {
    // Rust crate targets use underscores, and EnvFilter target directives match
    // prefixes, so this includes every moli workspace crate but no dependency.
    let filter = first_party_filter(log_filter);

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .log_internal_errors(false)
        .try_init();
}

#[cfg(test)]
mod tests {
    use std::{
        io::{self, Write},
        sync::Arc,
    };

    use parking_lot::Mutex;
    use tracing_subscriber::fmt::MakeWriter;

    use super::first_party_filter;

    #[derive(Clone, Default)]
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    struct SharedWriterGuard(Arc<Mutex<Vec<u8>>>);

    #[derive(Clone, Copy)]
    struct BrokenWriter;

    impl Write for SharedWriterGuard {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.0.lock().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Write for BrokenWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "diagnostic sink is gone",
            ))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "diagnostic sink is gone",
            ))
        }
    }

    impl<'writer> MakeWriter<'writer> for BrokenWriter {
        type Writer = Self;

        fn make_writer(&'writer self) -> Self::Writer {
            *self
        }
    }

    impl<'writer> MakeWriter<'writer> for SharedWriter {
        type Writer = SharedWriterGuard;

        fn make_writer(&'writer self) -> Self::Writer {
            SharedWriterGuard(Arc::clone(&self.0))
        }
    }

    impl SharedWriter {
        fn output(&self) -> String {
            String::from_utf8(self.0.lock().clone()).unwrap()
        }
    }

    #[test]
    fn first_party_filter_excludes_dependency_and_lower_level_logs() {
        let writer = SharedWriter::default();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_target(true)
            .with_writer(writer.clone())
            .with_env_filter(first_party_filter("info"))
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "moli_parser", "first-party-info");
            tracing::debug!(target: "moli_parser", "first-party-debug");
            tracing::warn!(target: "html5ever::tree_builder", "dependency-warning");
        });

        let output = writer.output();
        assert!(output.contains("first-party-info"));
        assert!(!output.contains("first-party-debug"));
        assert!(!output.contains("dependency-warning"));
    }

    #[test]
    fn non_level_filter_falls_back_to_first_party_info() {
        let writer = SharedWriter::default();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_target(true)
            .with_writer(writer.clone())
            .with_env_filter(first_party_filter("info,dependency=trace"))
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "moli", "fallback-info");
            tracing::warn!(target: "dependency", "dependency-warning");
        });

        let output = writer.output();
        assert!(output.contains("fallback-info"));
        assert!(!output.contains("dependency-warning"));
    }

    #[test]
    fn broken_diagnostic_writer_does_not_panic() {
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_target(false)
            .with_writer(BrokenWriter)
            .with_env_filter(first_party_filter("error"))
            .log_internal_errors(false)
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            tracing::error!("broken diagnostic writer regression");
        });
    }
}
