# Compression dictionary fixtures

The `dictionary.*` and `payload.*` Brotli, gzip, Zstandard, dcb and dcz bytes are copied unchanged from the upstream WPT `fetch/compression-dictionary/resources/register-dictionary.py` and `compressed-data.py`, snapshot `db95fafd1fcef8428805e41eb5705d444e8c67ce`. The two deflate fixtures encode the same dictionary with Python zlib, with and without the zlib wrapper.

Dictionary bytes are `This is a test dictionary.\n`; payload bytes are `This is compressed test data using a test dictionary`. The dcb/dcz headers and dictionary SHA-256 remain part of each fixture.
