# Chromium LevelDB test ports

The source is Chromium's actual LevelDB dependency, pinned to
[`7ee830d02b623e8ffe0b95d59a74db1e58da04c5`](https://chromium.googlesource.com/external/leveldb/+/7ee830d02b623e8ffe0b95d59a74db1e58da04c5/).
Chromium revision
[`d9fa5a15858b7189aa1a70cebe811169154ebcf0`](https://chromium.googlesource.com/chromium/src/+/d9fa5a15858b7189aa1a70cebe811169154ebcf0/third_party/leveldatabase/)
selects that gitlink and identifies it as LevelDB 1.23 in
[`README.chromium`](https://chromium.googlesource.com/chromium/src/+/d9fa5a15858b7189aa1a70cebe811169154ebcf0/third_party/leveldatabase/README.chromium).
The tests retain the upstream [license](LICENSE-LevelDB) and
[authors](AUTHORS-LevelDB).

The suite has 133 tests: 68 ports/adaptations of the scenarios listed below,
56 additional parser regressions, and the 9 original snapshot tests. This is
coverage of the read-only parser, not the complete Chromium LevelDB engine
suite. Each original scenario keeps its snake_case name, with a suffix where
its observable behavior was adapted. Supporting fixtures are test-only code.

```sh
cargo test -p moli-leveldb-parser -p moli-cookie-import --lib
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --no-fail-fast
```

## WAL physical records

Source: [`db/log_test.cc`](https://chromium.googlesource.com/external/leveldb/+/7ee830d02b623e8ffe0b95d59a74db1e58da04c5/db/log_test.cc).
Port: [`src/tests/log.rs`](src/tests/log.rs), 31 tests, including these 23 upstream cases:

| Upstream cases | What is checked |
| --- | --- |
| `Empty`, `ReadWrite`, `ManyBlocks` | Empty records and 100,000 records over many physical blocks |
| `Fragmentation`, `RandomRead` | 50/100 KB values and 500 skewed-size records, fixed seed 301 |
| `MarginalTrailer`, `MarginalTrailer2`, `ShortTrailer`, `AlignedEof` | Exact block ends, short padding and empty FIRST fragments |
| `OpenForAppend` | Reading a log whose writer resumed at a nonzero block offset |
| `ReadError` | An injected I/O error propagates |
| `BadRecordType`, `BadLength`, `ChecksumMismatch` | Unknown types, records crossing a block, and bad CRC32C |
| `UnexpectedMiddleType`, `UnexpectedLastType`, `UnexpectedFullType`, `UnexpectedFirstType` | Invalid fragment ordering |
| `TruncatedTrailingRecordIsIgnored`, `BadLengthAtEndIsIgnored`, `MissingLastIsIgnored`, `PartialLastIsIgnored` | Torn final appends never emit partial records |
| `ErrorJoinsRecords` | Corruption must not join fragments belonging to different logical records |

Upstream's reader reports corruption and can continue salvaging records. The
parser returns `Err` at complete corruption; the public snapshot API exposes
no partial result. Tests assert errors instead of upstream dropped-byte counts
and salvaged data. Incomplete final appends retain upstream EOF behavior.

The remaining 15 upstream cases (`SkipIntoMultiRecord`, `ReadStart`,
`ReadSecond*`, `ReadThird*`, `ReadFourth*`, `ReadInitialOffsetIntoBlockPadding`,
`ReadEnd`, `ReadPastEnd`) exercise an initial-offset/seek API. This parser always
starts at byte zero, so those API-specific cases are outside its scope.

Additional tests cover the empty-FIRST compatibility exception documented in
[`db/log_reader.cc`](https://chromium.googlesource.com/external/leveldb/+/7ee830d02b623e8ffe0b95d59a74db1e58da04c5/db/log_reader.cc),
checksums of every fragment kind, zero-filled preallocation, corrupt padding,
short/interrupted reads, callback errors, and cuts around every fragment boundary.

## Coding, CRC and WriteBatch

The four additional [layout tests](src/tests/layout.rs) use literal wire bytes
at every alignment modulo 8 to check borrowed headers, trailers and restart
arrays, little-endian values, truncation without cursor advancement, and
nonempty blocks with zero restarts. Pointer checks ensure views refer to the
original input; the existing corruption tests continue to validate semantics.

| Source | Ported cases | Local tests |
| --- | --- | --- |
| [`util/coding_test.cc`](https://chromium.googlesource.com/external/leveldb/+/7ee830d02b623e8ffe0b95d59a74db1e58da04c5/util/coding_test.cc) | All 10: `Fixed32`, `Fixed64`, `EncodingOutput`, `Varint32`, `Varint64`, both overflow cases, both truncation cases, `Strings` | [coding.rs](src/tests/coding.rs) |
| [`util/crc32c_test.cc`](https://chromium.googlesource.com/external/leveldb/+/7ee830d02b623e8ffe0b95d59a74db1e58da04c5/util/crc32c_test.cc) | `StandardResults`, `Values`, `Mask` | [coding.rs](src/tests/coding.rs) |
| [`db/write_batch_test.cc`](https://chromium.googlesource.com/external/leveldb/+/7ee830d02b623e8ffe0b95d59a74db1e58da04c5/db/write_batch_test.cc) | `Empty`, `Multiple`, `Corruption`, `Append` | [write_batch.rs](src/tests/write_batch.rs) |

`EncodingOutput` checks little-endian decoding against literal bytes; the
parser has no encoder. `CRC.Mask` checks the production masking wrapper
against its unmasked checksum. `CRC.Extend` and `WriteBatch.ApproximateSize`
test APIs absent from this crate. `Append` checks the combined on-disk batch;
the parser retains the latest version per key rather than all memtable history.

Added regressions cover truncated fixed fields and length prefixes, overlong
varint32 encodings, high bits that would overflow, every batch truncation point,
counts too small/large, unsupported tags, 56-bit sequence limits, empty keys and
values, ordered deletes, and conflicting values at the same sequence.

## MANIFEST / VersionEdit

Source: [`db/version_edit_test.cc`](https://chromium.googlesource.com/external/leveldb/+/7ee830d02b623e8ffe0b95d59a74db1e58da04c5/db/version_edit_test.cc).
Port: [`src/tests/manifest.rs`](src/tests/manifest.rs), 16 tests.

`VersionEditTest.EncodeDecode` preserves the large `(1 << 50)` file, size and
sequence values, additions, removals and compaction pointers. Assertions check
decoded manifest state instead of encoding it again. Its arbitrary `foo`
comparator is tested as an explicit unsupported-comparator error; supported
snapshots use `leveldb.BytewiseComparator`.

Additional cases cover metadata split across records, missing metadata,
previous-log updates, every VersionEdit field's truncation, unknown tags,
invalid levels/internal keys, sequence overflow, file deletion and level moves,
deletions-before-additions in one edit, CURRENT validation/selection, incomplete
final edits, and complete edits with invalid checksums. `ComparatorCheck` also
corresponds to the custom-comparator mismatch scenario in `db/db_test.cc`.

## SST blocks and internal keys

Sources: [`table/table_test.cc`](https://chromium.googlesource.com/external/leveldb/+/7ee830d02b623e8ffe0b95d59a74db1e58da04c5/table/table_test.cc),
[`db/dbformat_test.cc`](https://chromium.googlesource.com/external/leveldb/+/7ee830d02b623e8ffe0b95d59a74db1e58da04c5/db/dbformat_test.cc).
Port: [`src/tests/table.rs`](src/tests/table.rs), 23 tests.

| Upstream cases | Adaptation |
| --- | --- |
| `Harness.Empty`, `SimpleEmptyKey`, `SimpleSingle`, `SimpleMulti`, `SimpleSpecialKey` | Block and table forward scans; restart intervals 1/16/1024; raw and Snappy data/index blocks |
| `Harness.ZeroRestartPointsInBlock` | The four-byte empty block emitted by Java LevelDB |
| `Harness.Randomized` | Original 0..2000 entry-count schedule, binary-key alphabet and skewed lengths; fixed seed 306 |
| `Harness.RandomizedLongDB` | Checked-in native fixture from 100,000 writes, fixed seed 301; Rust independently reconstructs all 25,479 final entries |
| `TableTest.ApproximateOffsetOfPlain` | Original values up to 300 KB; verifies full contents instead of an offset-estimation API |
| `CompressionTableTest.ApproximateOffsetOfCompressed` | Original 25%-compressible workload; verifies Snappy-decoded contents |
| `FormatTest.InternalKey_EncodeDecode`, `InternalKey_DecodeFromEmpty` | Snapshot decoding of empty/long keys and sequence boundaries; rejection of short internal keys |

The read-only API exposes a full scan. Reverse/custom comparators, backward
iterators, seeks, memtable constructors, key-shortening/debug formatting, and
approximate offsets have no corresponding API here. The pinned Chromium
[`port_chromium.h`](https://chromium.googlesource.com/chromium/src/+/d9fa5a15858b7189aa1a70cebe811169154ebcf0/third_party/leveldatabase/port/port_chromium.h)
returns false for Zstd support, so the Zstd parameter is not ported; compression
types other than raw/Snappy are explicitly rejected.

Added regressions cover footer size/magic, file-size disagreement with MANIFEST,
out-of-bounds block handles, data/index CRCs, invalid compression with valid
CRCs, restart counts/offsets/order, illegal shared prefixes, entries crossing
restart arrays, unknown internal value types, and callback errors. Existing
snapshot tests also cover malicious Snappy lengths and missing/legacy tables.

## Recovery, versions and tombstones

Sources: [`db/recovery_test.cc`](https://chromium.googlesource.com/external/leveldb/+/7ee830d02b623e8ffe0b95d59a74db1e58da04c5/db/recovery_test.cc),
[`db/db_test.cc`](https://chromium.googlesource.com/external/leveldb/+/7ee830d02b623e8ffe0b95d59a74db1e58da04c5/db/db_test.cc),
[`db/corruption_test.cc`](https://chromium.googlesource.com/external/leveldb/+/7ee830d02b623e8ffe0b95d59a74db1e58da04c5/db/corruption_test.cc).
Port: [`src/tests/recovery.rs`](src/tests/recovery.rs), 21 tests.

| Upstream cases | Read-only expectation |
| --- | --- |
| `RecoveryTest.ManifestReused`, `LargeManifestCompacted` | Appended edits and a 3 MB zero-padded MANIFEST read successfully; source files stay unchanged |
| `RecoveryTest.NoLogFiles`, `LogFileReuse` | Missing logs on a valid snapshot and empty/nonempty current WALs |
| `RecoveryTest.MultipleMemTables` | Original 1,000 padded keys recovered entirely in memory |
| `RecoveryTest.MultipleLogFiles` | Recover newer unregistered WALs while ignoring obsolete logs, even with larger sequences |
| `RecoveryTest.ManifestMissing` | Fail without creating/replacing files |
| `DBTest.GetLevel0Ordering`, `GetOrderedByLevels` | Newest visible version wins across overlapping L0 files and different levels |
| `DBTest.Recover`, `RecoveryWithEmptyLog` | Repeated updates across WAL/SST transitions |
| `DBTest.IterMultiWithDelete`, `IterMultiWithDeleteAndCompaction` | Deleted entries absent from output with both WAL-only and SST+WAL data |
| `CorruptionTest.SequenceNumberRecovery` | Recovered sequence ordering keeps subsequent updates visible; repair itself is outside scope |
| `CorruptionTest.Recovery` | Corruption in the original two WAL locations returns an error instead of salvaged partial data |

These are 15 upstream scenarios. Added regressions cover native Chromium
SST/WAL compatibility, future SST versions, tombstones newer than WAL values,
empty batches, malformed batches without partial output, and duplicate numeric
WAL filenames. Recovery checks compare the input files before and after reads.

The nine existing [snapshot tests](src/tests/snapshot.rs) moved from the Chrome
importer. They retain engine comparison, mtime/content checks, a held writer
lock, read-only filesystem permissions, obsolete-file resurrection checks,
`.sst` fallback/missing live tables (`DBTest.StillReadSST`/`MissingSSTFile`
scenarios), corruption, previous WAL selection, and UTF-agnostic byte handling.
Chrome string/JSON import tests remain in `moli-cookie-import`.

Native fixtures, their C++ generator, checksums and regeneration instructions
are in [tests/fixtures](tests/fixtures/README.md). Normal tests require no C++
compiler or network. Compaction scheduling, writable recovery/repair,
concurrency, caches/Bloom-filter performance, locking policy, and write-failure
injection for a database engine are outside the parser's API and are not
claimed as ported.
