// Copyright 2026 the Moli authors. MIT license.

#include "support.h"
#include "v8-message.h"

using namespace support;

extern "C" {

const v8::Data* v8__Message__GetHostDefinedOptions(const v8::Message& self) {
  return local_to_ptr(self.GetScriptOrigin().GetHostDefinedOptions());
}

}  // extern "C"
