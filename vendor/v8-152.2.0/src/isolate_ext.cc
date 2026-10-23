// Copyright 2026 the Moli authors. MIT license.

#include "v8-callbacks.h"
#include "v8-isolate.h"

extern "C" {

void v8__Isolate__SetIsJSApiWrapperNativeErrorCallback(
    v8::Isolate* isolate, v8::IsJSApiWrapperNativeErrorCallback callback) {
  isolate->SetIsJSApiWrapperNativeErrorCallback(callback);
}

}  // extern "C"
