CREATE TABLE database_metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    name BLOB NOT NULL CHECK (length(name) % 2 = 0),
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
