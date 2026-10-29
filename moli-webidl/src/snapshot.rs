//! Typed dictionary snapshots. Only parsed Rust fields and native private slots
//! can create the carrier accepted by the copy path. Author JS objects cannot
//! accidentally be reparsed or invoke their getters during snapshotting.
//!
//! Parsing already produces Rust scalar/container values. JS/interface leaves
//! remain scoped V8 handles; snapshots move long-lived ownership into the
//! receiver's V8 graph. A generic `into_native` that stores independent Global
//! roots would retain otherwise collectible owner/interface cycles, including
//! across retained Realms. CommonRuntimeState remains appropriate for native
//! handles and API algorithm state, with its existing owner-lifetime protocol.
//!
//! Renderer migrations use the dictionary schema for RTC configuration,
//! WebCodecs support results, WebAuthn JSON decoding, WebMCP tool snapshots,
//! inherited event payloads, Touch, DOMPoint/DOMRect, ImageData settings,
//! StaticRange, errors, text decoders, queuing strategies, observer options,
//! ClipboardItem options and Trusted Types policy callbacks. Native producers
//! reuse those typed dictionaries instead of a second JS field declaration.
//!
//! Input-only dictionaries retain their parser without opting into output:
//! fetch/body/header normalization, Blob/File byte and MIME processing,
//! permissions/storage/OPFS/service-worker commands, credential validation,
//! media/geolocation/worklet options, CSS/font parsing, transform math, observer
//! scheduling and compact modifier bitsets. Streams' source/sink/transformer
//! dictionaries feed native algorithms and their existing invocation carriers;
//! they are not persisted or returned as dictionary snapshots. Structured-clone
//! algorithms (e.g. Performance detail) keep their API-specific clone policy.

mod value;

use moli_v8_util::{get_private_value, new_null_prototype_object, private_key, v8_string, v8str};
use moli_webapi_declare::WebApiValue;
use std::marker::PhantomData;

#[derive(Clone, Copy)]
pub enum SnapshotPrototype {
    Null,
    Ordinary,
}

#[derive(Clone, Copy)]
pub enum SnapshotKind {
    /// Primitive values; containers are rejected instead of silently aliased.
    Scalar,
    /// Opaque JS values/interfaces whose identity must be preserved.
    Identity,
    Sequence(&'static SnapshotKind),
    FrozenSequence(&'static SnapshotKind),
    Dictionary(&'static SnapshotSchema),
    Nullable(&'static SnapshotKind),
    Record(&'static SnapshotKind),
    BufferSource,
    /// API-specific policy for an already converted native value.
    Custom(
        for<'s, 'i> fn(
            &mut v8::PinScope<'s, 'i>,
            v8::Local<'s, v8::Value>,
            SnapshotPrototype,
        ) -> Option<v8::Local<'s, v8::Value>>,
    ),
}

pub struct SnapshotMember {
    pub name: &'static str,
    pub kind: SnapshotKind,
    pub optional: bool,
}

pub struct SnapshotSchema {
    pub inherited: Option<&'static SnapshotSchema>,
    pub members: &'static [SnapshotMember],
}

/// Raw/custom converters explicitly describe their canonical JS representation.
pub trait WebIdlSnapshotValue {
    const SNAPSHOT_KIND: SnapshotKind;
}
pub trait WebIdlDictionarySnapshot {
    const SNAPSHOT_SCHEMA: SnapshotSchema;
}

/// Writes converted Rust fields without reading author object properties.
pub trait WebIdlDictionarySerialize<'s>: WebIdlDictionarySnapshot + Sized {
    fn write_snapshot_own_fields(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        writer: &SnapshotWriter<'s>,
    ) -> Option<()>;

    fn write_snapshot_fields(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        writer: &SnapshotWriter<'s>,
    ) -> Option<()>;

    fn to_js_snapshot(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        prototype: SnapshotPrototype,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let writer = SnapshotWriter {
            object: new_object(scope, prototype),
            prototype,
            state: false,
        };
        self.write_snapshot_fields(scope, &writer)?;
        Some(writer.object)
    }

    fn to_snapshot(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<DictionarySnapshot<'s, Self>> {
        let object = self.to_js_snapshot(scope, SnapshotPrototype::Null)?;
        Some(DictionarySnapshot {
            object,
            marker: PhantomData,
        })
    }

    /// Install fields on an existing native backing. Slot mappings apply here;
    /// dictionary results always use their original WebIDL member names.
    fn initialize_snapshot(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Option<()> {
        self.write_snapshot_fields(
            scope,
            &SnapshotWriter {
                object,
                prototype: SnapshotPrototype::Null,
                state: true,
            },
        )
    }

    /// Legacy reinitializers update this dictionary's payload without resetting
    /// inherited event state that their own algorithms have already initialized.
    fn initialize_own_snapshot(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Option<()> {
        self.write_snapshot_own_fields(
            scope,
            &SnapshotWriter {
                object,
                prototype: SnapshotPrototype::Null,
                state: true,
            },
        )
    }
}

/// Traced internal state. No constructor accepts an arbitrary JS object.
pub struct DictionarySnapshot<'s, T: WebIdlDictionarySnapshot> {
    object: v8::Local<'s, v8::Object>,
    marker: PhantomData<fn() -> T>,
}

impl<'s, T: WebIdlDictionarySnapshot> DictionarySnapshot<'s, T> {
    pub fn from_private_slot(
        scope: &mut v8::PinScope<'s, '_>,
        owner: v8::Local<'s, v8::Object>,
        slot: &'static str,
    ) -> Option<Self> {
        let object = get_private_value(scope, owner, slot)?.try_into().ok()?;
        Some(Self {
            object,
            marker: PhantomData,
        })
    }

    pub fn store_private(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: v8::Local<'s, v8::Object>,
        slot: &'static str,
    ) -> Option<()> {
        let key = private_key(scope, slot)?;
        owner
            .set_private(scope, key, self.object.into())?
            .then_some(())
    }

    pub fn to_js_snapshot(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        prototype: SnapshotPrototype,
    ) -> Option<v8::Local<'s, v8::Object>> {
        copy_dictionary(scope, self.object, &T::SNAPSHOT_SCHEMA, prototype)
    }
}

impl<'s, T: WebIdlDictionarySnapshot> WebApiValue<'s> for DictionarySnapshot<'s, T> {
    fn to_v8_value(&self, _: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        Some(self.object.into())
    }
}

/// Sink used by generated serializers for dictionary results and backing state.
pub struct SnapshotWriter<'s> {
    object: v8::Local<'s, v8::Object>,
    pub prototype: SnapshotPrototype,
    state: bool,
}

const CALLBACK_CONTEXTS: &str = "__moliDictionarySnapshotCallbackContexts";

impl<'s> SnapshotWriter<'s> {
    /// Native state hooks may store invocation carriers while JS snapshots
    /// retain the original callback/interface identity.
    pub const fn is_state(&self) -> bool {
        self.state
    }

    /// Converted callbacks retain their conversion-time contexts in the V8
    /// owner graph, never in a process- or runtime-owned Global root.
    pub fn anchor_contexts(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        contexts: &[v8::Local<'s, v8::Object>],
    ) -> Option<()> {
        let key = private_key(scope, CALLBACK_CONTEXTS)?;
        let array = match get_private_value(scope, self.object, CALLBACK_CONTEXTS) {
            Some(value) => v8::Local::<v8::Array>::try_from(value).ok()?,
            None => {
                let array = v8::Array::new(scope, 0);
                array
                    .set_prototype(scope, v8::null(scope).into())?
                    .then_some(())?;
                self.object
                    .set_private(scope, key, array.into())?
                    .then_some(())?;
                array
            }
        };
        for context in contexts {
            moli_webapi_declare::define_array_data_property(
                scope,
                array,
                array.length(),
                (*context).into(),
            )?;
        }
        Some(())
    }

    pub fn set(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        name: &'static str,
        slot: Option<&str>,
        value: v8::Local<'s, v8::Value>,
        attributes: v8::PropertyAttribute,
    ) -> Option<()> {
        if self.state
            && let Some(slot) = slot
        {
            let key = private_key(scope, slot)?;
            self.object.set_private(scope, key, value)?.then_some(())
        } else if self.state && attributes != v8::PropertyAttribute::NONE {
            self.object
                .define_own_property(scope, v8str(scope, name).into(), value, attributes)?
                .then_some(())
        } else {
            self.object
                .create_data_property(scope, v8str(scope, name).into(), value)?
                .then_some(())
        }
    }
}

fn new_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: SnapshotPrototype,
) -> v8::Local<'s, v8::Object> {
    match prototype {
        SnapshotPrototype::Null => new_null_prototype_object(scope),
        SnapshotPrototype::Ordinary => v8::Object::new(scope),
    }
}

// Inspect an own data descriptor. Even a corrupted native carrier must not run
// an accessor or consult a polluted prototype during a snapshot read.
fn data_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: v8::Local<'s, v8::Name>,
) -> Option<v8::Local<'s, v8::Value>> {
    let descriptor = object.get_own_property_descriptor(scope, key)?;
    let descriptor = v8::Local::<v8::Object>::try_from(descriptor).ok()?;
    moli_v8_util::get_own_static_property(scope, descriptor, "value")
}

fn copy_dictionary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    schema: &'static SnapshotSchema,
    prototype: SnapshotPrototype,
) -> Option<v8::Local<'s, v8::Object>> {
    let output = new_object(scope, prototype);
    copy_members(scope, source, output, schema, prototype)?;
    if matches!(prototype, SnapshotPrototype::Null)
        && let Some(contexts) = get_private_value(scope, source, CALLBACK_CONTEXTS)
    {
        let key = private_key(scope, CALLBACK_CONTEXTS)?;
        output.set_private(scope, key, contexts)?.then_some(())?;
    }
    Some(output)
}

fn copy_members<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    output: v8::Local<'s, v8::Object>,
    schema: &'static SnapshotSchema,
    prototype: SnapshotPrototype,
) -> Option<()> {
    if let Some(base) = schema.inherited {
        copy_members(scope, source, output, base, prototype)?;
    }
    for member in schema.members {
        let key = v8str(scope, member.name);
        if !source.has_own_property(scope, key.into())? {
            if member.optional {
                continue;
            }
            return None;
        }
        let value = data_property(scope, source, key.into())?;
        let value = member.kind.copy(scope, value, prototype)?;
        output
            .create_data_property(scope, key.into(), value)?
            .then_some(())?;
    }
    Some(())
}

impl SnapshotKind {
    pub fn copy<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        source: v8::Local<'s, v8::Value>,
        prototype: SnapshotPrototype,
    ) -> Option<v8::Local<'s, v8::Value>> {
        match self {
            Self::Scalar => (!source.is_object()).then_some(source),
            Self::Identity => Some(source),
            Self::Nullable(_) if source.is_null() => Some(source),
            Self::Nullable(kind) => kind.copy(scope, source, prototype),
            Self::Sequence(kind) | Self::FrozenSequence(kind) => {
                let source = v8::Local::<v8::Array>::try_from(source).ok()?;
                let mut items = Vec::with_capacity(source.length() as usize);
                for index in 0..source.length() {
                    let key = v8_string(scope, &index.to_string())?;
                    let item = data_property(scope, source.into(), key.into())?;
                    items.push(kind.copy(scope, item, prototype)?);
                }
                let output = v8::Array::new_with_elements(scope, &items);
                if matches!(self, Self::FrozenSequence(_)) {
                    output
                        .set_integrity_level(scope, v8::IntegrityLevel::Frozen)?
                        .then_some(())?;
                }
                Some(output.into())
            }
            Self::Dictionary(schema) => {
                copy_dictionary(scope, source.try_into().ok()?, schema, prototype).map(Into::into)
            }
            Self::Record(kind) => {
                let source = v8::Local::<v8::Object>::try_from(source).ok()?;
                let output = new_object(scope, prototype);
                let keys = source.get_own_property_names(scope, Default::default())?;
                for index in 0..keys.length() {
                    let key =
                        v8::Local::<v8::Name>::try_from(keys.get_index(scope, index)?).ok()?;
                    let value = data_property(scope, source, key)?;
                    let value = kind.copy(scope, value, prototype)?;
                    output
                        .create_data_property(scope, key, value)?
                        .then_some(())?;
                }
                Some(output.into())
            }
            Self::BufferSource => buffer_snapshot(scope, source),
            Self::Custom(copy) => copy(scope, source, prototype),
        }
    }
}

pub fn buffer_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let bytes = match crate::convert::<crate::BufferSource>(
        scope,
        source,
        crate::Context::member("DictionarySnapshot", "buffer"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            crate::throw_error(scope, &error);
            return None;
        }
    };
    Some(buffer_from_bytes(scope, bytes))
}

fn buffer_from_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
) -> v8::Local<'s, v8::Value> {
    let store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    v8::ArrayBuffer::with_backing_store(scope, &store).into()
}

#[doc(hidden)]
pub fn bytes_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: &[u8],
) -> v8::Local<'s, v8::Value> {
    buffer_from_bytes(scope, bytes.to_vec())
}
