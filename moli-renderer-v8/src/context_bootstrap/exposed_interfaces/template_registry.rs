use std::cell::{Cell, RefCell};
use std::rc::Rc;

use anyhow::{Result, anyhow};

use super::metadata::{
    ExposedInterfaceMetadata, ExposedInterfaceMetadataTable, GlobalInstallation, InterfaceId,
    TemplateBuildProfile,
};
use crate::context_bootstrap::build_profiled_exposed_interface_template;
use crate::context_bootstrap::specs::ConstructorSpec;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TemplateBuildReason {
    Direct,
    Parent,
}

enum IsolateTemplateState {
    Uninitialized,
    Building,
    Ready(v8::Global<v8::FunctionTemplate>),
    Failed(String),
}

struct ExposedInterfaceTemplateEntry {
    state: RefCell<IsolateTemplateState>,
    build_attempt_count: Cell<usize>,
    build_count: Cell<usize>,
    direct_build_count: Cell<usize>,
    parent_build_count: Cell<usize>,
    materialization_count: Cell<usize>,
}

impl ExposedInterfaceTemplateEntry {
    fn new() -> Self {
        Self {
            state: RefCell::new(IsolateTemplateState::Uninitialized),
            build_attempt_count: Cell::new(0),
            build_count: Cell::new(0),
            direct_build_count: Cell::new(0),
            parent_build_count: Cell::new(0),
            materialization_count: Cell::new(0),
        }
    }
}

pub(in crate::context_bootstrap) struct ExposedInterfaceTemplateRegistry {
    metadata: ExposedInterfaceMetadataTable,
    specs: Vec<ConstructorSpec>,
    profile: TemplateBuildProfile,
    templates: Vec<ExposedInterfaceTemplateEntry>,
}

impl ExposedInterfaceTemplateRegistry {
    pub(in crate::context_bootstrap) fn install<C>(
        scope: &mut v8::PinScope<'_, '_, C>,
        specs: Vec<ConstructorSpec>,
        profile: TemplateBuildProfile,
    ) -> Result<Rc<Self>> {
        let metadata = ExposedInterfaceMetadataTable::from_constructor_specs(&specs)?;
        moli_webapi_declare::register_web_api_interfaces(
            scope,
            specs.iter().map(|spec| (spec.name, spec.parent)),
        )?;
        let templates = (0..metadata.len())
            .map(|_| ExposedInterfaceTemplateEntry::new())
            .collect();
        let registry = Rc::new(Self {
            metadata,
            specs,
            profile,
            templates,
        });
        scope
            .as_mut()
            .set_slot(registry.clone())
            .then_some(registry)
            .ok_or_else(|| anyhow!("exposed interface template registry was already installed"))
    }

    pub(super) fn current(scope: &mut v8::PinScope<'_, '_>) -> Option<Rc<Self>> {
        scope.get_slot::<Rc<Self>>().cloned()
    }

    pub(super) fn metadata(&self, id: InterfaceId) -> Option<ExposedInterfaceMetadata> {
        self.metadata.get(id)
    }

    pub(super) fn metadata_entries(&self) -> &[ExposedInterfaceMetadata] {
        self.metadata.entries()
    }

    pub(in crate::context_bootstrap) fn id_by_name(&self, name: &str) -> Option<InterfaceId> {
        self.metadata.id_by_name(name)
    }

    pub(super) fn is_lazy_name(&self, name: &str) -> bool {
        self.metadata
            .id_by_name(name)
            .and_then(|id| self.metadata.get(id))
            .is_some_and(|metadata| metadata.installation == GlobalInstallation::Lazy)
    }

    pub(super) fn supports_interface(&self, id: InterfaceId) -> bool {
        self.metadata
            .get(id)
            .is_some_and(|metadata| metadata.is_supported_by(self.profile))
    }

    pub(super) fn len(&self) -> usize {
        self.metadata.len()
    }

    pub(in crate::context_bootstrap) fn get_or_build_template<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        id: InterfaceId,
    ) -> Result<v8::Local<'s, v8::FunctionTemplate>> {
        self.get_or_build_template_with_reason(scope, id, TemplateBuildReason::Direct)
    }

    fn get_or_build_template_with_reason<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        id: InterfaceId,
        reason: TemplateBuildReason,
    ) -> Result<v8::Local<'s, v8::FunctionTemplate>> {
        let metadata = self
            .metadata(id)
            .ok_or_else(|| anyhow!("unknown exposed interface id {}", id.index()))?;
        if !metadata.is_supported_by(self.profile) {
            return Err(anyhow!(
                "interface `{}` is not supported by {:?} template profile",
                metadata.name,
                self.profile
            ));
        }
        let entry = self
            .templates
            .get(id.index())
            .ok_or_else(|| anyhow!("template state is out of range for `{}`", metadata.name))?;
        {
            let state = entry.state.borrow();
            match &*state {
                IsolateTemplateState::Ready(template) => {
                    return Ok(v8::Local::new(scope, template));
                }
                IsolateTemplateState::Building => {
                    return Err(anyhow!(
                        "template dependency cycle reached `{}`",
                        metadata.name
                    ));
                }
                IsolateTemplateState::Failed(message) => {
                    return Err(anyhow!(
                        "a previous template build for `{}` failed: {message}",
                        metadata.name
                    ));
                }
                IsolateTemplateState::Uninitialized => {}
            }
        }

        *entry.state.borrow_mut() = IsolateTemplateState::Building;
        entry
            .build_attempt_count
            .set(entry.build_attempt_count.get() + 1);
        let started = std::time::Instant::now();
        let result = self.build_uninitialized_template(scope, id, metadata);
        match result {
            Ok(template) => {
                let global = v8::Global::new(scope.as_ref(), template);
                *entry.state.borrow_mut() = IsolateTemplateState::Ready(global);
                entry.build_count.set(entry.build_count.get() + 1);
                match reason {
                    TemplateBuildReason::Direct => {
                        entry
                            .direct_build_count
                            .set(entry.direct_build_count.get() + 1);
                    }
                    TemplateBuildReason::Parent => {
                        entry
                            .parent_build_count
                            .set(entry.parent_build_count.get() + 1);
                    }
                }
                tracing::debug!(
                    target: "moli_webapi_template",
                    interface = metadata.name,
                    profile = ?self.profile,
                    reason = ?reason,
                    elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
                    "built exposed interface FunctionTemplate"
                );
                Ok(template)
            }
            Err(error) => {
                *entry.state.borrow_mut() = IsolateTemplateState::Failed(error.to_string());
                Err(error)
            }
        }
    }

    fn build_uninitialized_template<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        id: InterfaceId,
        metadata: ExposedInterfaceMetadata,
    ) -> Result<v8::Local<'s, v8::FunctionTemplate>> {
        let parent = metadata
            .parent
            .map(|parent| {
                self.get_or_build_template_with_reason(scope, parent, TemplateBuildReason::Parent)
            })
            .transpose()?;
        let spec = self.specs.get(id.index()).copied().ok_or_else(|| {
            anyhow!(
                "missing constructor spec for exposed interface `{}`",
                metadata.name
            )
        })?;
        if spec.name != metadata.name {
            return Err(anyhow!(
                "constructor spec `{}` does not match metadata `{}`",
                spec.name,
                metadata.name
            ));
        }
        let template = build_profiled_exposed_interface_template(scope, spec, self.profile)?;
        if let Some(parent) = parent {
            template.inherit(parent);
        }
        Ok(template)
    }

    pub(super) fn ready_template<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        id: InterfaceId,
    ) -> Option<v8::Local<'s, v8::FunctionTemplate>> {
        let entry = self.templates.get(id.index())?;
        let state = entry.state.borrow();
        let IsolateTemplateState::Ready(template) = &*state else {
            return None;
        };
        Some(v8::Local::new(scope, template))
    }

    pub(super) fn record_materialization(&self, id: InterfaceId) {
        if let Some(entry) = self.templates.get(id.index()) {
            entry
                .materialization_count
                .set(entry.materialization_count.get() + 1);
        }
    }

    #[cfg(test)]
    pub(super) fn build_count(&self, id: InterfaceId) -> usize {
        self.templates
            .get(id.index())
            .map_or(0, |entry| entry.build_count.get())
    }

    #[cfg(test)]
    pub(super) fn ready_template_count(&self) -> usize {
        self.templates
            .iter()
            .filter(|entry| matches!(*entry.state.borrow(), IsolateTemplateState::Ready(_)))
            .count()
    }

    #[cfg(test)]
    pub(super) fn ready_template_names(&self) -> Vec<&'static str> {
        self.metadata_entries()
            .iter()
            .filter(|metadata| {
                self.templates
                    .get(metadata.id.index())
                    .is_some_and(|entry| {
                        matches!(*entry.state.borrow(), IsolateTemplateState::Ready(_))
                    })
            })
            .map(|metadata| metadata.name)
            .collect()
    }

    #[cfg(test)]
    pub(super) fn materialization_count(&self, id: InterfaceId) -> usize {
        self.templates
            .get(id.index())
            .map_or(0, |entry| entry.materialization_count.get())
    }
}
