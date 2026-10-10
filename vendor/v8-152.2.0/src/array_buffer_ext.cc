// Copyright 2026 the Moli authors. MIT license.

#include "support.h"
#include "v8-array-buffer.h"

using namespace support;

extern "C" {

bool v8__ArrayBuffer__IsResizableByUserJavaScript(const v8::ArrayBuffer& self) {
  return ptr_to_local(&self)->IsResizableByUserJavaScript();
}

}  // extern "C"
