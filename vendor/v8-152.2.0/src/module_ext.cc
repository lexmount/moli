// Copyright 2026 the Moli authors. MIT license.

#include "support.h"
#include "v8.h"

using namespace support;

extern "C" {

const v8::Value* v8__Module__GetResourceName(const v8::Module& self) {
  return local_to_ptr(self.GetResourceName());
}

MaybeBool v8__Module__SetSyntheticModuleExportUninitialized(
    const v8::Module& self,
    v8::Isolate* isolate,
    const v8::String* export_name) {
  // Use the isolate root slot as the Local handle storage. Copying the tagged
  // root value can misrepresent static roots in prebuilt V8 configurations.
  auto root_slot = reinterpret_cast<const v8::Value* const*>(
      v8::internal::Internals::GetRootSlot(
          isolate, v8::internal::Internals::kTheHoleValueRootIndex));
  auto value = *reinterpret_cast<const v8::Local<v8::Value>*>(&root_slot);
  return maybe_to_maybe_bool(ptr_to_local(&self)->SetSyntheticModuleExport(
      isolate, ptr_to_local(export_name), value));
}

}  // extern "C"
