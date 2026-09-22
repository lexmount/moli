# Engineering reliability verification

Locally validated source: `22e161c85c399420b2fb1957d5c3ee9bba822a66`, based on `de1093d9c1ab23a0bf9b1c7d0ee99141c2eee134`. Published source: `f13e9f427ebd14e6f68f36d6f7df5add29e85ac1`, rebased onto main `7241328085c8fdf0e18cbd6a2ef4d0eec878efb5`. `rebase-provenance.json` verifies the feature patch is byte-for-byte unchanged; CI validates the published head.

- `rust-gates.json`: source tree, required commands, exit codes, test summary, build settings, and original log hashes.
- `ci-script-tests.json`: 13 CI script tests, tested file hashes, and command.
- `stderr-red-green.json`: identical real release CLI scenario with a writable or deliberately closed stderr pipe, compared between base and head.

This partition does not own any of the historical 34 failed replay cases. Its acceptance criteria are reliable browser diagnostics, archive validation, and portable test fixtures. The full replay cohort is recorded separately; a change in replay outcome alone does not establish causality.
