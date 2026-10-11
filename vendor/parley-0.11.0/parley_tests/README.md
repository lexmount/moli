# parley_tests

Integration test suite for the Parley text layout library.

## Overview

This crate contains the integration test suite for `parley`. It follows a similar architecture to
[`vello_sparse_tests`](https://github.com/linebender/vello/tree/main/sparse_strips/vello_sparse_tests) and uses snapshot
testing to validate layout correctness.

## Running Tests

```bash
$ cargo test -p parley_tests
```

If a test fails, you can compare images in /parley/tests/current (images created by the current test)
and /parley/tests/snapshots (the accepted versions).

If you think that everything is ok, you can start tests as follows:

```bash
$ PARLEY_TEST="accept" cargo test -p parley_tests
```

It will update snapshots of the failed tests.

## Usage of xtask when a test fails

After some tests fail, you may run the following for generating a Kompari HTML report:

```bash
cargo xtask report
```

or start an interactive test blessing

```bash
cargo xtask review
```

## Detect dead snapshots

The following command shows snapshots that are not used in any test. The command also allows to delete such snapshots.

```bash
cargo xtask dead-snapshots
```

## Matching Chrome in line height calculations

The `lines.html` file can be used to generate images roughly similar to the ones in `test_lines.rs`.
It can be used to verify existing test goals or to create additional tests.
