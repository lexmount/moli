// Copyright (c) 2011 The LevelDB Authors. All rights reserved.
// RandomKey/RandomString and the stress workload are adapted from LevelDB's
// util/testutil.cc and table/table_test.cc. See ../../LICENSE-LevelDB,
// ../../AUTHORS-LevelDB and ../../TESTS.md.
//
// Build against the pinned LevelDB source as documented in README.md here.
// This generator is not compiled or run by Cargo.

#include <cstdio>
#include <filesystem>
#include <iostream>
#include <map>
#include <memory>
#include <stdexcept>
#include <string>

#include "leveldb/db.h"
#include "leveldb/write_batch.h"
#include "util/random.h"

using Entries = std::map<std::string, std::string>;

static void Check(const leveldb::Status& status) {
  if (!status.ok()) throw std::runtime_error(status.ToString());
}

static std::string RandomKey(leveldb::Random& random, int length) {
  static const char chars[] = {'\0', '\1', 'a', 'b', 'c', 'd', 'e',
                               '\xfd', '\xfe', '\xff'};
  std::string key;
  for (int i = 0; i < length; ++i) key += chars[random.Uniform(sizeof(chars))];
  return key;
}

static std::string RandomString(leveldb::Random& random, int length) {
  std::string value;
  for (int i = 0; i < length; ++i) value += char(' ' + random.Uniform(95));
  return value;
}

static std::unique_ptr<leveldb::DB> Open(const std::filesystem::path& path) {
  leveldb::Options options;
  options.create_if_missing = true;
  options.error_if_exists = true;
  options.write_buffer_size = 10000;
  options.block_size = 256;
  options.compression = leveldb::kNoCompression;
  leveldb::DB* db = nullptr;
  Check(leveldb::DB::Open(options, path.string(), &db));
  return std::unique_ptr<leveldb::DB>(db);
}

static void Verify(leveldb::DB& db, const Entries& expected) {
  auto iter = std::unique_ptr<leveldb::Iterator>(db.NewIterator(leveldb::ReadOptions()));
  iter->SeekToFirst();
  for (const auto& [key, value] : expected) {
    if (!iter->Valid() || iter->key().ToString() != key || iter->value().ToString() != value)
      throw std::runtime_error("native iterator differs from expected contents");
    iter->Next();
  }
  Check(iter->status());
  if (iter->Valid()) throw std::runtime_error("native iterator has extra entries");
}

static void RandomizedLongDB(const std::filesystem::path& path) {
  auto db = Open(path);
  leveldb::Random random(301);
  Entries expected;
  for (int i = 0; i < 100000; ++i) {
    // Explicit evaluation order matches the Rust port on every C++ compiler.
    const int key_length = random.Skewed(4);
    const auto key = RandomKey(random, key_length);
    const int value_length = random.Skewed(5);
    const auto value = RandomString(random, value_length);
    Check(db->Put(leveldb::WriteOptions(), key, value));
    expected[key] = value;
  }
  Verify(*db, expected);
  std::cout << path.filename() << ": " << expected.size() << " entries\n";
}

static void Recovery(const std::filesystem::path& path) {
  auto db = Open(path);
  Entries expected;
  for (int i = 0; i < 200; ++i) {
    char key[16]; std::snprintf(key, sizeof(key), "key-%04d", i);
    const std::string value = "value-" + std::to_string(i) + std::string(300, 'x');
    Check(db->Put(leveldb::WriteOptions(), key, value));
    expected[key] = value;
  }
  Check(db->Put(leveldb::WriteOptions(), "deleted", "must-not-return"));
  Check(db->Put(leveldb::WriteOptions(), "large", std::string(100000, 'x')));
  Check(db->Put(leveldb::WriteOptions(), "", "")); expected[""] = "";
  const std::string binary_key("\0\xffk", 3), binary_value("\xff\0", 2);
  Check(db->Put(leveldb::WriteOptions(), binary_key, binary_value));
  expected[binary_key] = binary_value;
  db->CompactRange(nullptr, nullptr);

  leveldb::WriteBatch batch;
  for (int i = 0; i < 200; ++i) {
    char key[16]; std::snprintf(key, sizeof(key), "key-%04d", i);
    if (i % 7 == 0) { batch.Delete(key); expected.erase(key); }
    else if (i % 5 == 0) { batch.Put(key, "new"); expected[key] = "new"; }
  }
  batch.Delete("deleted");
  batch.Put("large", std::string(120000, 'y'));
  expected["large"] = std::string(120000, 'y');
  leveldb::WriteOptions write; write.sync = true;
  Check(db->Write(write, &batch));
  Verify(*db, expected);
  std::cout << path.filename() << ": " << expected.size() << " entries\n";
}

int main(int argc, char** argv) {
  try {
    if (argc != 2) throw std::runtime_error("usage: generate OUTPUT_DIRECTORY");
    const std::filesystem::path root(argv[1]);
    if (!std::filesystem::create_directory(root))
      throw std::runtime_error("output directory must not already exist");
    RandomizedLongDB(root / "randomized-long-db");
    Recovery(root / "recovery");
    // Keep only the closed database's format-bearing files. Removing LOCK/LOG
    // here does not change snapshot contents; the Rust tests never open writers.
    for (const auto& directory : std::filesystem::directory_iterator(root)) {
      for (const auto& entry : std::filesystem::directory_iterator(directory)) {
        const auto name = entry.path().filename().string();
        if (name == "LOCK" || name == "LOG" || name == "LOG.old")
          std::filesystem::remove(entry);
      }
    }
  } catch (const std::exception& error) {
    std::cerr << error.what() << '\n';
    return 1;
  }
}
