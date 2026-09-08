use std::fmt;

use url::Url;

use crate::document_runtime::DomHandle;
use crate::frame_owner_model::{FrameDocumentOwner, FrameRealmId};
use crate::script_vm::ScriptVm;

use super::{
    ModuleEntryId, ModuleFetchMetadata, ModuleIdentityHash, ModuleLoadError,
    ModuleMapFetchDisposition, ModuleMapKey, ModuleRecordEntry, ModuleRequestRecord,
    ModuleResolvedDependency, ModuleSource, NativeDocumentModulator,
    NativeModuleMapSingleModuleClient,
};

pub(super) struct NativeModuleTreeDocumentOwner<'a> {
    vm: &'a mut ScriptVm,
    compile_frame_realm: Option<FrameRealmId>,
}

pub(crate) struct NativeModuleTreeFrameDocumentOwner<'a> {
    vm: &'a mut ScriptVm,
    document_modulator: &'a mut NativeDocumentModulator,
    document_owner: FrameDocumentOwner,
    realm_id: FrameRealmId,
    module_request_initiator_url: Url,
}

pub(crate) trait NativeModuleTreeDocumentOwnerAdapter {
    fn compile_module_record(
        &mut self,
        key: ModuleMapKey,
        source: &ModuleSource,
        source_url: &Url,
        fetch_metadata: &ModuleFetchMetadata,
    ) -> std::result::Result<(ModuleRecordEntry, ModuleIdentityHash), ModuleLoadError>;

    fn start_or_join_module_fetch(&mut self, key: ModuleMapKey) -> ModuleMapFetchDisposition;

    fn resolve_module_specifier(
        &mut self,
        specifier: &str,
        base_url: &Url,
    ) -> std::result::Result<Url, String>;

    fn resolve_module_integrity(&self, url: &Url) -> Option<String>;

    fn module_request_initiator_url(&self, child_handle: Option<DomHandle>) -> Url;

    fn dispatch_module_fetch_csp_report_only_violation(
        &mut self,
        key: &ModuleMapKey,
        fetch_metadata: &ModuleFetchMetadata,
    );

    fn csp_blocked_module_fetch_error(
        &mut self,
        key: &ModuleMapKey,
        fetch_metadata: &ModuleFetchMetadata,
    ) -> Option<ModuleLoadError>;

    fn suspend_module_fetch_waiter(
        &mut self,
        key: ModuleMapKey,
        client: NativeModuleMapSingleModuleClient,
    );

    fn module_source(&self, entry: ModuleEntryId) -> Option<ModuleSource>;

    fn module_failure(&self, entry: ModuleEntryId) -> Option<ModuleLoadError>;

    fn module_entry_id(&self, key: &ModuleMapKey) -> Option<ModuleEntryId>;

    fn module_entry_state(&self, entry: ModuleEntryId) -> super::ModuleMapEntryState;

    fn insert_module_source_for_request(
        &mut self,
        request_key: ModuleMapKey,
        effective_key: ModuleMapKey,
        source: ModuleSource,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) -> ModuleEntryId;

    fn insert_compiled_module_record(
        &mut self,
        request_key: ModuleMapKey,
        record: ModuleRecordEntry,
        identity: ModuleIdentityHash,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) -> ModuleEntryId;

    fn module_entry_key(&self, entry: ModuleEntryId) -> ModuleMapKey;

    fn module_entry_url(&self, entry: ModuleEntryId) -> Url;

    fn module_effective_fetch_metadata(&self, entry: ModuleEntryId) -> ModuleFetchMetadata;

    fn module_requests(&self, entry: ModuleEntryId) -> Vec<ModuleRequestRecord>;

    fn set_module_resolved_dependencies(
        &mut self,
        entry: ModuleEntryId,
        dependencies: Vec<ModuleResolvedDependency>,
    );

    fn mark_module_failed(&mut self, key: ModuleMapKey, error: ModuleLoadError) -> ModuleEntryId;

    fn preserve_module_load_error(
        &mut self,
        error: ModuleLoadError,
    ) -> std::result::Result<ModuleLoadError, ModuleLoadError>;

    fn record_runtime_warning(&mut self, message: fmt::Arguments<'_>);
}

impl<T: NativeModuleTreeDocumentOwnerAdapter + ?Sized> NativeModuleTreeDocumentOwnerAdapter
    for &mut T
{
    fn compile_module_record(
        &mut self,
        key: ModuleMapKey,
        source: &ModuleSource,
        source_url: &Url,
        fetch_metadata: &ModuleFetchMetadata,
    ) -> std::result::Result<(ModuleRecordEntry, ModuleIdentityHash), ModuleLoadError> {
        (*self).compile_module_record(key, source, source_url, fetch_metadata)
    }

    fn start_or_join_module_fetch(&mut self, key: ModuleMapKey) -> ModuleMapFetchDisposition {
        (*self).start_or_join_module_fetch(key)
    }

    fn resolve_module_specifier(
        &mut self,
        specifier: &str,
        base_url: &Url,
    ) -> std::result::Result<Url, String> {
        (*self).resolve_module_specifier(specifier, base_url)
    }

    fn resolve_module_integrity(&self, url: &Url) -> Option<String> {
        (**self).resolve_module_integrity(url)
    }

    fn module_request_initiator_url(&self, child_handle: Option<DomHandle>) -> Url {
        (**self).module_request_initiator_url(child_handle)
    }

    fn dispatch_module_fetch_csp_report_only_violation(
        &mut self,
        key: &ModuleMapKey,
        fetch_metadata: &ModuleFetchMetadata,
    ) {
        (*self).dispatch_module_fetch_csp_report_only_violation(key, fetch_metadata);
    }

    fn csp_blocked_module_fetch_error(
        &mut self,
        key: &ModuleMapKey,
        fetch_metadata: &ModuleFetchMetadata,
    ) -> Option<ModuleLoadError> {
        (*self).csp_blocked_module_fetch_error(key, fetch_metadata)
    }

    fn suspend_module_fetch_waiter(
        &mut self,
        key: ModuleMapKey,
        client: NativeModuleMapSingleModuleClient,
    ) {
        (*self).suspend_module_fetch_waiter(key, client);
    }

    fn module_source(&self, entry: ModuleEntryId) -> Option<ModuleSource> {
        (**self).module_source(entry)
    }

    fn module_failure(&self, entry: ModuleEntryId) -> Option<ModuleLoadError> {
        (**self).module_failure(entry)
    }

    fn module_entry_id(&self, key: &ModuleMapKey) -> Option<ModuleEntryId> {
        (**self).module_entry_id(key)
    }

    fn module_entry_state(&self, entry: ModuleEntryId) -> super::ModuleMapEntryState {
        (**self).module_entry_state(entry)
    }

    fn insert_module_source_for_request(
        &mut self,
        request_key: ModuleMapKey,
        effective_key: ModuleMapKey,
        source: ModuleSource,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) -> ModuleEntryId {
        (*self).insert_module_source_for_request(
            request_key,
            effective_key,
            source,
            effective_fetch_metadata,
        )
    }

    fn insert_compiled_module_record(
        &mut self,
        request_key: ModuleMapKey,
        record: ModuleRecordEntry,
        identity: ModuleIdentityHash,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) -> ModuleEntryId {
        (*self).insert_compiled_module_record(
            request_key,
            record,
            identity,
            effective_fetch_metadata,
        )
    }

    fn module_entry_key(&self, entry: ModuleEntryId) -> ModuleMapKey {
        (**self).module_entry_key(entry)
    }

    fn module_entry_url(&self, entry: ModuleEntryId) -> Url {
        (**self).module_entry_url(entry)
    }

    fn module_effective_fetch_metadata(&self, entry: ModuleEntryId) -> ModuleFetchMetadata {
        (**self).module_effective_fetch_metadata(entry)
    }

    fn module_requests(&self, entry: ModuleEntryId) -> Vec<ModuleRequestRecord> {
        (**self).module_requests(entry)
    }

    fn set_module_resolved_dependencies(
        &mut self,
        entry: ModuleEntryId,
        dependencies: Vec<ModuleResolvedDependency>,
    ) {
        (*self).set_module_resolved_dependencies(entry, dependencies);
    }

    fn mark_module_failed(&mut self, key: ModuleMapKey, error: ModuleLoadError) -> ModuleEntryId {
        (*self).mark_module_failed(key, error)
    }

    fn preserve_module_load_error(
        &mut self,
        error: ModuleLoadError,
    ) -> std::result::Result<ModuleLoadError, ModuleLoadError> {
        (*self).preserve_module_load_error(error)
    }

    fn record_runtime_warning(&mut self, message: fmt::Arguments<'_>) {
        (*self).record_runtime_warning(message);
    }
}

impl<'a> NativeModuleTreeDocumentOwner<'a> {
    pub(super) fn new(vm: &'a mut ScriptVm) -> Self {
        Self::new_with_compile_frame_realm(vm, None)
    }

    pub(super) fn new_with_compile_frame_realm(
        vm: &'a mut ScriptVm,
        compile_frame_realm: Option<FrameRealmId>,
    ) -> Self {
        Self {
            vm,
            compile_frame_realm,
        }
    }
}

impl<'a> NativeModuleTreeFrameDocumentOwner<'a> {
    pub(crate) fn new(
        vm: &'a mut ScriptVm,
        document_modulator: &'a mut NativeDocumentModulator,
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        module_request_initiator_url: Url,
    ) -> Self {
        Self {
            vm,
            document_modulator,
            document_owner,
            realm_id,
            module_request_initiator_url,
        }
    }
}

impl NativeModuleTreeDocumentOwnerAdapter for NativeModuleTreeDocumentOwner<'_> {
    fn preserve_module_load_error(
        &mut self,
        error: ModuleLoadError,
    ) -> std::result::Result<ModuleLoadError, ModuleLoadError> {
        self.vm
            .preserve_native_module_load_error(self.compile_frame_realm, error)
    }

    fn compile_module_record(
        &mut self,
        key: ModuleMapKey,
        source: &ModuleSource,
        source_url: &Url,
        fetch_metadata: &ModuleFetchMetadata,
    ) -> std::result::Result<(ModuleRecordEntry, ModuleIdentityHash), ModuleLoadError> {
        if let Some(realm_id) = self.compile_frame_realm {
            return self.vm.compile_native_module_record_for_frame_realm(
                realm_id,
                key,
                source,
                source_url,
                fetch_metadata,
            );
        }
        self.vm
            .compile_native_module_record(key, source, source_url, fetch_metadata)
    }

    fn start_or_join_module_fetch(&mut self, key: ModuleMapKey) -> ModuleMapFetchDisposition {
        self.vm
            .document_runtime
            .start_or_join_native_module_fetch(key)
    }

    fn resolve_module_specifier(
        &mut self,
        specifier: &str,
        base_url: &Url,
    ) -> std::result::Result<Url, String> {
        super::resolve_module_specifier(self.vm, specifier, base_url)
    }

    fn resolve_module_integrity(&self, url: &Url) -> Option<String> {
        super::resolve_module_integrity(&*self.vm, url)
    }

    fn module_request_initiator_url(&self, child_handle: Option<DomHandle>) -> Url {
        child_handle
            .and_then(|handle| {
                self.vm
                    .child_browsing_context_module_request_initiator_url(handle)
            })
            .unwrap_or_else(|| self.vm.document_runtime.document_url().clone())
    }

    fn dispatch_module_fetch_csp_report_only_violation(
        &mut self,
        key: &ModuleMapKey,
        fetch_metadata: &ModuleFetchMetadata,
    ) {
        self.vm
            .dispatch_module_fetch_csp_report_only_violation_for_owner(key, fetch_metadata);
    }

    fn csp_blocked_module_fetch_error(
        &mut self,
        key: &ModuleMapKey,
        fetch_metadata: &ModuleFetchMetadata,
    ) -> Option<ModuleLoadError> {
        self.vm
            .csp_blocked_module_fetch_error_for_owner(key, fetch_metadata)
    }

    fn suspend_module_fetch_waiter(
        &mut self,
        key: ModuleMapKey,
        client: NativeModuleMapSingleModuleClient,
    ) {
        self.vm
            .document_runtime
            .suspend_native_module_fetch_waiter(key, client);
    }

    fn module_source(&self, entry: ModuleEntryId) -> Option<ModuleSource> {
        self.vm.document_runtime.native_module_source(entry)
    }

    fn module_failure(&self, entry: ModuleEntryId) -> Option<ModuleLoadError> {
        self.vm.document_runtime.native_module_failure(entry)
    }

    fn module_entry_id(&self, key: &ModuleMapKey) -> Option<ModuleEntryId> {
        self.vm.document_runtime.native_module_entry_id(key)
    }

    fn module_entry_state(&self, entry: ModuleEntryId) -> super::ModuleMapEntryState {
        self.vm.document_runtime.native_module_entry_state(entry)
    }

    fn insert_module_source_for_request(
        &mut self,
        request_key: ModuleMapKey,
        effective_key: ModuleMapKey,
        source: ModuleSource,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) -> ModuleEntryId {
        self.vm
            .document_runtime
            .insert_native_module_source_for_request(
                request_key,
                effective_key,
                source,
                effective_fetch_metadata,
            )
    }

    fn insert_compiled_module_record(
        &mut self,
        request_key: ModuleMapKey,
        record: ModuleRecordEntry,
        identity: ModuleIdentityHash,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) -> ModuleEntryId {
        self.vm
            .document_runtime
            .insert_native_compiled_module_record_with_metadata(
                request_key,
                record,
                identity,
                effective_fetch_metadata,
            )
    }

    fn module_entry_key(&self, entry: ModuleEntryId) -> ModuleMapKey {
        self.vm.document_runtime.native_module_entry_key(entry)
    }

    fn module_entry_url(&self, entry: ModuleEntryId) -> Url {
        self.vm.document_runtime.native_module_entry_url(entry)
    }

    fn module_effective_fetch_metadata(&self, entry: ModuleEntryId) -> ModuleFetchMetadata {
        self.vm
            .document_runtime
            .native_module_effective_fetch_metadata(entry)
    }

    fn module_requests(&self, entry: ModuleEntryId) -> Vec<ModuleRequestRecord> {
        self.vm
            .document_runtime
            .native_module_requests(entry)
            .unwrap_or_default()
    }

    fn set_module_resolved_dependencies(
        &mut self,
        entry: ModuleEntryId,
        dependencies: Vec<ModuleResolvedDependency>,
    ) {
        self.vm
            .document_runtime
            .set_native_module_resolved_dependencies(entry, dependencies);
    }

    fn mark_module_failed(&mut self, key: ModuleMapKey, error: ModuleLoadError) -> ModuleEntryId {
        self.vm
            .document_runtime
            .mark_native_module_failed(key, error)
    }

    fn record_runtime_warning(&mut self, message: fmt::Arguments<'_>) {
        super::driver::record_runtime_warning(self.vm, message);
    }
}

impl NativeModuleTreeDocumentOwnerAdapter for NativeModuleTreeFrameDocumentOwner<'_> {
    fn preserve_module_load_error(
        &mut self,
        error: ModuleLoadError,
    ) -> std::result::Result<ModuleLoadError, ModuleLoadError> {
        self.vm
            .preserve_native_module_load_error(Some(self.realm_id), error)
    }

    fn compile_module_record(
        &mut self,
        key: ModuleMapKey,
        source: &ModuleSource,
        source_url: &Url,
        fetch_metadata: &ModuleFetchMetadata,
    ) -> std::result::Result<(ModuleRecordEntry, ModuleIdentityHash), ModuleLoadError> {
        self.vm.compile_native_module_record_for_frame_realm(
            self.realm_id,
            key,
            source,
            source_url,
            fetch_metadata,
        )
    }

    fn start_or_join_module_fetch(&mut self, key: ModuleMapKey) -> ModuleMapFetchDisposition {
        self.document_modulator.start_or_join_fetch(key)
    }

    fn resolve_module_specifier(
        &mut self,
        specifier: &str,
        base_url: &Url,
    ) -> std::result::Result<Url, String> {
        self.vm.resolve_child_frame_module_specifier(
            self.document_owner,
            self.realm_id,
            specifier,
            base_url,
        )
    }

    fn resolve_module_integrity(&self, url: &Url) -> Option<String> {
        self.vm
            .resolve_child_frame_module_integrity(self.document_owner, self.realm_id, url)
    }

    fn module_request_initiator_url(&self, _child_handle: Option<DomHandle>) -> Url {
        self.module_request_initiator_url.clone()
    }

    fn dispatch_module_fetch_csp_report_only_violation(
        &mut self,
        _key: &ModuleMapKey,
        _fetch_metadata: &ModuleFetchMetadata,
    ) {
    }

    fn csp_blocked_module_fetch_error(
        &mut self,
        _key: &ModuleMapKey,
        _fetch_metadata: &ModuleFetchMetadata,
    ) -> Option<ModuleLoadError> {
        None
    }

    fn suspend_module_fetch_waiter(
        &mut self,
        key: ModuleMapKey,
        client: NativeModuleMapSingleModuleClient,
    ) {
        self.document_modulator
            .add_single_module_fetch_client(key, client);
    }

    fn module_source(&self, entry: ModuleEntryId) -> Option<ModuleSource> {
        self.document_modulator.entry(entry).source().cloned()
    }

    fn module_failure(&self, entry: ModuleEntryId) -> Option<ModuleLoadError> {
        self.document_modulator.entry(entry).failure().cloned()
    }

    fn module_entry_id(&self, key: &ModuleMapKey) -> Option<ModuleEntryId> {
        self.document_modulator.entry_id(key)
    }

    fn module_entry_state(&self, entry: ModuleEntryId) -> super::ModuleMapEntryState {
        self.document_modulator.entry(entry).state()
    }

    fn insert_module_source_for_request(
        &mut self,
        request_key: ModuleMapKey,
        effective_key: ModuleMapKey,
        source: ModuleSource,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) -> ModuleEntryId {
        self.document_modulator.insert_fetched_source_for_request(
            request_key,
            effective_key,
            source,
            effective_fetch_metadata,
        )
    }

    fn insert_compiled_module_record(
        &mut self,
        request_key: ModuleMapKey,
        record: ModuleRecordEntry,
        identity: ModuleIdentityHash,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) -> ModuleEntryId {
        self.document_modulator
            .insert_compiled_record_with_metadata(
                request_key,
                record,
                identity,
                effective_fetch_metadata,
            )
    }

    fn module_entry_key(&self, entry: ModuleEntryId) -> ModuleMapKey {
        self.document_modulator.entry(entry).effective_key().clone()
    }

    fn module_entry_url(&self, entry: ModuleEntryId) -> Url {
        self.document_modulator.entry_url(entry)
    }

    fn module_effective_fetch_metadata(&self, entry: ModuleEntryId) -> ModuleFetchMetadata {
        self.document_modulator
            .entry(entry)
            .effective_fetch_metadata()
            .clone()
    }

    fn module_requests(&self, entry: ModuleEntryId) -> Vec<ModuleRequestRecord> {
        self.document_modulator
            .compiled_record(entry)
            .map(|record| record.requests().to_vec())
            .unwrap_or_default()
    }

    fn set_module_resolved_dependencies(
        &mut self,
        entry: ModuleEntryId,
        dependencies: Vec<ModuleResolvedDependency>,
    ) {
        self.document_modulator
            .set_resolved_dependencies(entry, dependencies);
    }

    fn mark_module_failed(&mut self, key: ModuleMapKey, error: ModuleLoadError) -> ModuleEntryId {
        self.document_modulator.mark_failed(key, error)
    }

    fn record_runtime_warning(&mut self, message: fmt::Arguments<'_>) {
        self.vm.record_runtime_warning(message);
    }
}
