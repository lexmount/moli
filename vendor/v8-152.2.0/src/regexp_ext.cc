// Copyright 2026 the Moli authors. MIT license.

#include "support.h"
#include "v8-regexp.h"

using namespace support;

extern "C" {

const v8::RegExp* v8__RegExp__NewWithBacktrackLimit(
    const v8::Context& context,
    const v8::String& pattern,
    v8::RegExp::Flags flags,
    uint32_t backtrack_limit) {
  // The V8 API aborts on invalid limits. Keep Rust's fallible constructor
  // fallible for these values as well, including pointer-compressed builds.
  if (backtrack_limit == 0 ||
      backtrack_limit > static_cast<uint32_t>(v8::internal::kSmiMaxValue)) {
    return nullptr;
  }
  return maybe_local_to_ptr(v8::RegExp::NewWithBacktrackLimit(
      ptr_to_local(&context), ptr_to_local(&pattern), flags, backtrack_limit));
}

}  // extern "C"
