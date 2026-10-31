// Copyright 2026 the Moli authors. MIT license.

#include "support.h"
#include "v8-context.h"
#include "v8-isolate.h"

using namespace support;

extern "C" {

void v8__Isolate__SetSharedArrayBufferConstructorEnabledCallback(
    v8::Isolate* isolate,
    v8::SharedArrayBufferConstructorEnabledCallback callback) {
  isolate->SetSharedArrayBufferConstructorEnabledCallback(callback);
}

void v8__Isolate__InstallConditionalFeatures(v8::Isolate* isolate,
                                           const v8::Context& context) {
  isolate->InstallConditionalFeatures(ptr_to_local(&context));
}

}  // extern "C"
