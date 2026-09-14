// Copyright 2026 the Moli authors. MIT license.

#include "support.h"
#include "v8-function.h"

using namespace support;

extern "C" {

const v8::Value* v8__Function__GetBoundFunction(const v8::Function& self) {
  return local_to_ptr(ptr_to_local(&self)->GetBoundFunction());
}

}  // extern "C"
