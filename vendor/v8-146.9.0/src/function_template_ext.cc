// Copyright 2026 the Moli authors. MIT license.

#include "support.h"
#include "v8-object.h"
#include "v8-template.h"

using namespace support;

extern "C" {

const v8::FunctionTemplate* v8__FunctionTemplate__NewWithCache(
    v8::Isolate* isolate, v8::FunctionCallback callback,
    const v8::Private* cache_property, const v8::Value* data_or_null,
    const v8::Signature* signature_or_null, int length,
    v8::SideEffectType side_effect_type) {
  return local_to_ptr(v8::FunctionTemplate::NewWithCache(
      isolate, callback, ptr_to_local(cache_property),
      ptr_to_local(data_or_null), ptr_to_local(signature_or_null), length,
      side_effect_type));
}

void v8__Object__SetAccessorProperty(
    const v8::Object& self, const v8::Name& key,
    const v8::Function* getter_or_null,
    const v8::Function* setter_or_null, v8::PropertyAttribute attr) {
  ptr_to_local(&self)->SetAccessorProperty(ptr_to_local(&key),
                                           ptr_to_local(getter_or_null),
                                           ptr_to_local(setter_or_null), attr);
}

bool v8__FunctionTemplate__HasInstance(const v8::FunctionTemplate& self,
                                       const v8::Value& value) {
  return ptr_to_local(&self)->HasInstance(ptr_to_local(&value));
}

}  // extern "C"
