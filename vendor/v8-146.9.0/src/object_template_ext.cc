// Copyright 2026 the Moli authors. MIT license.

#include "support.h"
#include "v8-template.h"

using namespace support;

extern "C" {

void v8__ObjectTemplate__MarkAsUndetectable(const v8::ObjectTemplate& self) {
  ptr_to_local(&self)->MarkAsUndetectable();
}

void v8__ObjectTemplate__SetCodeLike(const v8::ObjectTemplate& self) {
  ptr_to_local(&self)->SetCodeLike();
}

void v8__Template__SetLazyDataProperty(
    const v8::Template& self, const v8::Name& key,
    v8::AccessorNameGetterCallback getter, const v8::Value* data_or_null,
    v8::PropertyAttribute attr, v8::SideEffectType getter_side_effect_type,
    v8::SideEffectType setter_side_effect_type) {
  ptr_to_local(&self)->SetLazyDataProperty(
      ptr_to_local(&key), getter, ptr_to_local(data_or_null), attr,
      getter_side_effect_type, setter_side_effect_type);
}

void v8__Template__SetPrivate(const v8::Template& self,
                              const v8::Private& key,
                              const v8::Data& value,
                              v8::PropertyAttribute attr) {
  ptr_to_local(&self)->SetPrivate(ptr_to_local(&key), ptr_to_local(&value),
                                  attr);
}

void v8__ObjectTemplate__SetCallAsFunctionHandler(
    const v8::ObjectTemplate& self, v8::FunctionCallback callback,
    const v8::Value* data_or_null) {
  ptr_to_local(&self)->SetCallAsFunctionHandler(callback,
                                                ptr_to_local(data_or_null));
}

void v8__ObjectTemplate__SetSecurityTokenAccessCheckAndHandlers(
    const v8::ObjectTemplate& self,
    v8::AccessCheckCallback access_check,
    v8::NamedPropertyGetterCallback named_getter,
    v8::NamedPropertySetterCallback named_setter,
    v8::NamedPropertyQueryCallback named_query,
    v8::NamedPropertyDeleterCallback named_deleter,
    v8::NamedPropertyEnumeratorCallback named_enumerator,
    v8::NamedPropertyDefinerCallback named_definer,
    v8::NamedPropertyDescriptorCallback named_descriptor,
    const v8::Value* named_data_or_null,
    v8::PropertyHandlerFlags named_flags,
    v8::IndexedPropertyGetterCallbackV2 indexed_getter,
    v8::IndexedPropertySetterCallbackV2 indexed_setter,
    v8::IndexedPropertyQueryCallbackV2 indexed_query,
    v8::IndexedPropertyDeleterCallbackV2 indexed_deleter,
    v8::IndexedPropertyEnumeratorCallback indexed_enumerator,
    v8::IndexedPropertyDefinerCallbackV2 indexed_definer,
    v8::IndexedPropertyDescriptorCallbackV2 indexed_descriptor,
    const v8::Value* indexed_data_or_null,
    v8::PropertyHandlerFlags indexed_flags) {
  ptr_to_local(&self)->SetAccessCheckCallbackAndHandler(
      access_check,
      v8::NamedPropertyHandlerConfiguration(
          named_getter, named_setter, named_query, named_deleter,
          named_enumerator, named_definer, named_descriptor,
          ptr_to_local(named_data_or_null), named_flags),
      v8::IndexedPropertyHandlerConfiguration(
          indexed_getter, indexed_setter, indexed_query, indexed_deleter,
          indexed_enumerator, indexed_definer, indexed_descriptor,
          ptr_to_local(indexed_data_or_null), indexed_flags));
}

}  // extern "C"
