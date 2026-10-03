use super::ScriptVm;
use moli_page_types::{
    DocumentActivity, DocumentSettings, EmulatedIdleOverride, EmulatedMediaOverrides,
    LayoutConfiguration, NavigatorOverrides, ViewportSurface,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocumentSettingsApplication {
    Bootstrap,
    Live,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocumentSettingEffect {
    Deferred,
    PublishLayout,
}

/// A single target setting. Borrowed values avoid cloning a whole snapshot
/// when a live command changes one property.
#[derive(Clone, Copy)]
pub(crate) enum DocumentSetting<'a> {
    ScriptExecutionDisabled(bool),
    ScrollbarsHidden(bool),
    BypassContentSecurityPolicy(bool),
    EmulatedMedia(&'a EmulatedMediaOverrides),
    IdleOverride(Option<EmulatedIdleOverride>),
    NavigatorOverrides(&'a NavigatorOverrides),
    ViewportSurface(Option<ViewportSurface>),
    DocumentActivity(DocumentActivity),
}

impl DocumentSetting<'_> {
    pub(crate) fn retain_in(self, settings: &mut DocumentSettings) {
        match self {
            Self::ScriptExecutionDisabled(value) => settings.script_execution_disabled = value,
            Self::ScrollbarsHidden(value) => settings.scrollbars_hidden = value,
            Self::BypassContentSecurityPolicy(value) => {
                settings.bypass_content_security_policy = value
            }
            Self::EmulatedMedia(value) => settings.emulated_media = value.clone(),
            Self::IdleOverride(value) => settings.idle_override = value,
            Self::NavigatorOverrides(value) => settings.navigator_overrides = value.clone(),
            Self::ViewportSurface(value) => settings.viewport_surface = value,
            Self::DocumentActivity(value) => settings.document_activity = value,
        }
    }
}

impl ScriptVm {
    /// Installs every setting before document-start scripts. Keep this
    /// destructure exhaustive so a new snapshot field must gain native setup.
    pub(crate) fn initialize_document_settings(
        &mut self,
        settings: &DocumentSettings,
        layout: LayoutConfiguration,
    ) -> anyhow::Result<()> {
        let DocumentSettings {
            script_execution_disabled,
            scrollbars_hidden,
            bypass_content_security_policy,
            emulated_media,
            idle_override,
            navigator_overrides,
            viewport_surface,
            document_activity,
        } = settings;
        for setting in [
            DocumentSetting::ScriptExecutionDisabled(*script_execution_disabled),
            DocumentSetting::BypassContentSecurityPolicy(*bypass_content_security_policy),
            DocumentSetting::EmulatedMedia(emulated_media),
            DocumentSetting::IdleOverride(*idle_override),
            DocumentSetting::NavigatorOverrides(navigator_overrides),
            DocumentSetting::ViewportSurface(*viewport_surface),
            DocumentSetting::DocumentActivity(*document_activity),
            DocumentSetting::ScrollbarsHidden(*scrollbars_hidden),
        ] {
            self.apply_document_setting(setting, layout, DocumentSettingsApplication::Bootstrap)?;
        }
        Ok(())
    }

    /// Bootstrap installs native inputs without environment events. Live
    /// updates keep each setting's event and geometry-publication boundary.
    pub(crate) fn apply_document_setting(
        &mut self,
        setting: DocumentSetting<'_>,
        layout: LayoutConfiguration,
        application: DocumentSettingsApplication,
    ) -> anyhow::Result<DocumentSettingEffect> {
        let live = application == DocumentSettingsApplication::Live;
        let mut effect = DocumentSettingEffect::Deferred;
        match setting {
            DocumentSetting::ScriptExecutionDisabled(value) => {
                self.set_script_execution_disabled(value)
            }
            DocumentSetting::BypassContentSecurityPolicy(value) => {
                self.set_bypass_content_security_policy(value)
            }
            DocumentSetting::EmulatedMedia(value) => {
                if live {
                    self.set_emulated_media(value);
                } else {
                    self.set_emulated_media_for_bootstrap(value);
                }
            }
            DocumentSetting::IdleOverride(value) => {
                if live {
                    self.set_idle_override_and_sync_surface(value)?;
                } else {
                    self.set_idle_override(value);
                }
            }
            DocumentSetting::NavigatorOverrides(value) => {
                if live {
                    self.set_navigator_overrides_and_sync_surface(value)?;
                } else {
                    self.set_navigator_overrides(value);
                }
            }
            DocumentSetting::ViewportSurface(value) => {
                if live {
                    self.set_viewport_surface(value)?;
                } else {
                    self.set_viewport_surface_for_bootstrap(value);
                }
            }
            DocumentSetting::DocumentActivity(value) => {
                if live {
                    self.set_document_activity(value)?;
                } else {
                    self.set_document_activity_for_bootstrap(value);
                }
            }
            DocumentSetting::ScrollbarsHidden(value) => {
                let hidden = layout.scrollbars_hidden_for(value);
                let changed = self._context_host.borrow().scrollbars_hidden() != hidden;
                self.set_scrollbars_hidden(hidden);
                if live && changed {
                    effect = DocumentSettingEffect::PublishLayout;
                }
            }
        }
        Ok(effect)
    }
}
