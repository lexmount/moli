# Quinn terminal-reader cleanup

This directory is the published `quinn` 0.11.12 crate. Its archive SHA-256 is
`4051e23e9185c255a7e33ef59cdbca87a22d359052eecd22fc6b901fb37d9d11`.

The only source changes backport the two terminal-reader cleanup hunks from
[upstream commit cdd79406522da30fbf01fdbbe305451dd6741cf0](https://github.com/quinn-rs/quinn/commit/cdd79406522da30fbf01fdbbe305451dd6741cf0).
Both successful EOF and terminal RESET remove a previously registered reader
waker. A cancelled `received_reset()` observer can otherwise leave that waker
behind when a subsequent read immediately consumes buffered data and FIN,
violating `RecvStream::drop`'s invariant and retaining stale task state.

The upstream waker-cloning optimization is not included. Moli's deterministic
regression is
`moli-webtransport::tests::stream_io::reading_buffered_fin_after_reset_observation_releases_the_native_reader`.
Remove this patch when a published compatible Quinn release includes the fix.
