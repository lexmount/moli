CREATE TABLE object_stores (
    store_id INTEGER PRIMARY KEY CHECK (store_id > 0),
    name BLOB NOT NULL CHECK (length(name) % 2 = 0) UNIQUE,
    key_path BLOB,
    auto_increment INTEGER NOT NULL CHECK (auto_increment IN (0, 1)),
    generator INTEGER NOT NULL CHECK (generator BETWEEN 0 AND 9007199254740992)
) STRICT, WITHOUT ROWID;

CREATE TABLE indexes (
    index_id INTEGER PRIMARY KEY CHECK (index_id > 0),
    store_id INTEGER NOT NULL CHECK (store_id > 0),
    name BLOB NOT NULL CHECK (length(name) % 2 = 0),
    key_path BLOB NOT NULL,
    is_unique INTEGER NOT NULL CHECK (is_unique IN (0, 1)),
    multi_entry INTEGER NOT NULL CHECK (multi_entry IN (0, 1)),
    UNIQUE (store_id, name)
) STRICT, WITHOUT ROWID;
