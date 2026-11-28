// Copyright 2026 the Moli authors. MIT license.

#include "v8-isolate.h"

extern "C" {

void v8__Isolate__AddCallCompletedCallback(
    v8::Isolate* isolate, v8::CallCompletedCallback callback) {
  isolate->AddCallCompletedCallback(callback);
}

void v8__Isolate__RemoveCallCompletedCallback(
    v8::Isolate* isolate, v8::CallCompletedCallback callback) {
  isolate->RemoveCallCompletedCallback(callback);
}

}  // extern "C"
