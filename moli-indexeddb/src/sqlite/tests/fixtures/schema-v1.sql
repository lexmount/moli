PRAGMA user_version = 1;

CREATE TABLE database_info (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    backend TEXT NOT NULL CHECK (backend = 'indexeddb'),
    owner_key BLOB NOT NULL CHECK (length(owner_key) > 0),
    identity_format INTEGER NOT NULL CHECK (identity_format = 1)
) STRICT, WITHOUT ROWID;

CREATE TABLE database_metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    name TEXT NOT NULL,
    generation BLOB NOT NULL CHECK (length(generation) = 16),
    version INTEGER NOT NULL CHECK (version >= 0),
    state TEXT NOT NULL CHECK (state IN ('creating', 'live')),
    key_format INTEGER NOT NULL CHECK (key_format = 1),
    key_path_format INTEGER NOT NULL CHECK (key_path_format = 1),
    value_format INTEGER NOT NULL CHECK (value_format = 1),
    usage INTEGER NOT NULL CHECK (usage >= 0),
    next_store_id INTEGER NOT NULL DEFAULT 1 CHECK (next_store_id > 0),
    next_index_id INTEGER NOT NULL DEFAULT 1 CHECK (next_index_id > 0),
    next_record_id INTEGER NOT NULL DEFAULT 1 CHECK (next_record_id > 0),
    next_object_id INTEGER NOT NULL DEFAULT 1 CHECK (next_object_id > 0),
    CHECK (
        (state = 'creating' AND version = 0)
        OR (state = 'live' AND version > 0)
    )
) STRICT, WITHOUT ROWID;

CREATE TABLE object_stores (
    store_id INTEGER PRIMARY KEY CHECK (store_id > 0),
    name TEXT NOT NULL UNIQUE,
    key_path BLOB,
    auto_increment INTEGER NOT NULL CHECK (auto_increment IN (0, 1)),
    generator INTEGER NOT NULL CHECK (generator BETWEEN 0 AND 9007199254740992)
) STRICT, WITHOUT ROWID;

CREATE TABLE indexes (
    index_id INTEGER PRIMARY KEY CHECK (index_id > 0),
    store_id INTEGER NOT NULL CHECK (store_id > 0),
    name TEXT NOT NULL,
    key_path BLOB NOT NULL,
    is_unique INTEGER NOT NULL CHECK (is_unique IN (0, 1)),
    multi_entry INTEGER NOT NULL CHECK (multi_entry IN (0, 1)),
    UNIQUE (store_id, name)
) STRICT, WITHOUT ROWID;

CREATE TABLE records (
    record_id INTEGER PRIMARY KEY CHECK (record_id > 0),
    store_id INTEGER NOT NULL CHECK (store_id > 0),
    key_sort BLOB NOT NULL CHECK (length(key_sort) > 0),
    key_value BLOB NOT NULL CHECK (length(key_value) > 0),
    wire_size INTEGER NOT NULL CHECK (wire_size >= 0),
    usage INTEGER NOT NULL CHECK (usage >= 0),
    UNIQUE (store_id, key_sort)
) STRICT, WITHOUT ROWID;

CREATE TABLE record_chunks (
    record_id INTEGER NOT NULL CHECK (record_id > 0),
    chunk_no INTEGER NOT NULL CHECK (chunk_no >= 0),
    bytes BLOB NOT NULL CHECK (length(bytes) BETWEEN 1 AND 262144),
    PRIMARY KEY (record_id, chunk_no)
) STRICT, WITHOUT ROWID;

CREATE TABLE index_entries (
    index_id INTEGER NOT NULL CHECK (index_id > 0),
    index_key_sort BLOB NOT NULL CHECK (length(index_key_sort) > 0),
    index_key_value BLOB NOT NULL CHECK (length(index_key_value) > 0),
    primary_key_sort BLOB NOT NULL CHECK (length(primary_key_sort) > 0),
    record_id INTEGER NOT NULL CHECK (record_id > 0),
    is_unique INTEGER NOT NULL CHECK (is_unique IN (0, 1)),
    PRIMARY KEY (index_id, index_key_sort, primary_key_sort)
) STRICT, WITHOUT ROWID;

CREATE UNIQUE INDEX index_entries_unique
    ON index_entries(index_id, index_key_sort) WHERE is_unique = 1;
CREATE INDEX index_entries_record ON index_entries(record_id);

CREATE TABLE external_objects (
    object_id INTEGER PRIMARY KEY CHECK (object_id > 0),
    kind TEXT NOT NULL CHECK (kind IN ('blob', 'file', 'file-system-handle')),
    size INTEGER NOT NULL CHECK (size >= 0),
    mime_type TEXT,
    file_name TEXT,
    last_modified_f64 BLOB CHECK (
        last_modified_f64 IS NULL OR length(last_modified_f64) = 8
    ),
    handle_kind TEXT CHECK (handle_kind IN ('file', 'directory')),
    handle_bucket_kind TEXT CHECK (handle_bucket_kind IN ('default', 'named')),
    handle_bucket_id INTEGER CHECK (handle_bucket_id IS NULL OR handle_bucket_id > 0),
    CHECK (
        (kind = 'blob' AND mime_type IS NOT NULL
         AND file_name IS NULL AND last_modified_f64 IS NULL
         AND handle_kind IS NULL AND handle_bucket_kind IS NULL AND handle_bucket_id IS NULL)
        OR
        (kind = 'file' AND mime_type IS NOT NULL
         AND file_name IS NOT NULL AND last_modified_f64 IS NOT NULL
         AND handle_kind IS NULL AND handle_bucket_kind IS NULL AND handle_bucket_id IS NULL)
        OR
        (kind = 'file-system-handle' AND mime_type IS NULL
         AND file_name IS NULL AND last_modified_f64 IS NULL AND size = 0
         AND handle_kind IS NOT NULL AND handle_bucket_kind IS NOT NULL
         AND ((handle_bucket_kind = 'default' AND handle_bucket_id IS NULL)
              OR (handle_bucket_kind = 'named' AND handle_bucket_id IS NOT NULL)))
    )
) STRICT, WITHOUT ROWID;

CREATE TABLE record_objects (
    record_id INTEGER NOT NULL CHECK (record_id > 0),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    object_id INTEGER NOT NULL CHECK (object_id > 0),
    PRIMARY KEY (record_id, ordinal)
) STRICT, WITHOUT ROWID;

CREATE INDEX record_objects_object ON record_objects(object_id);

CREATE TABLE blob_chunks (
    object_id INTEGER NOT NULL CHECK (object_id > 0),
    chunk_no INTEGER NOT NULL CHECK (chunk_no >= 0),
    bytes BLOB NOT NULL CHECK (length(bytes) BETWEEN 1 AND 262144),
    PRIMARY KEY (object_id, chunk_no)
) STRICT, WITHOUT ROWID;

CREATE TABLE handle_components (
    object_id INTEGER NOT NULL CHECK (object_id > 0),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    name TEXT NOT NULL,
    PRIMARY KEY (object_id, ordinal)
) STRICT, WITHOUT ROWID;
