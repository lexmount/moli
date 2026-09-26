// Copyright 2026 the Moli authors. MIT license.

#include "support.h"
#include "v8-inspector.h"

using namespace support;

extern "C" {

void v8_inspector__V8InspectorSession__resume(
    v8_inspector::V8InspectorSession* self, bool terminate_on_resume) {
  self->resume(terminate_on_resume);
}

void v8_inspector__V8InspectorSession__breakProgram(
    v8_inspector::V8InspectorSession* self,
    v8_inspector::StringView reason,
    v8_inspector::StringView detail) {
  self->breakProgram(reason, detail);
}

}  // extern "C"
