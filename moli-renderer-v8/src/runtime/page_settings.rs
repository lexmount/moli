use super::*;
use crate::script_vm::{DocumentSetting, DocumentSettingEffect, DocumentSettingsApplication};

impl PageVm {
    fn apply_document_setting(&mut self, setting: DocumentSetting<'_>) -> anyhow::Result<()> {
        let layout = self.layout_configuration;
        let effect = self.vm_mut().apply_document_setting(
            setting,
            layout,
            DocumentSettingsApplication::Live,
        )?;
        // Retain the target value before publication. Even if publication
        // fails, a followed navigation must inherit the accepted native input.
        setting.retain_in(&mut self.document_settings);
        if effect == DocumentSettingEffect::PublishLayout {
            self.vm_mut().publish_layout()?;
        }
        Ok(())
    }

    pub(crate) fn set_script_execution_disabled(&mut self, disabled: bool) -> anyhow::Result<()> {
        self.apply_document_setting(DocumentSetting::ScriptExecutionDisabled(disabled))
    }

    pub(crate) fn set_scrollbars_hidden(&mut self, hidden: bool) -> anyhow::Result<()> {
        self.apply_document_setting(DocumentSetting::ScrollbarsHidden(hidden))
    }

    pub(crate) fn set_bypass_content_security_policy(
        &mut self,
        bypass: bool,
    ) -> anyhow::Result<()> {
        self.apply_document_setting(DocumentSetting::BypassContentSecurityPolicy(bypass))
    }

    pub(crate) fn set_emulated_media(
        &mut self,
        overrides: &crate::protocol_types::EmulatedMediaOverrides,
    ) -> anyhow::Result<()> {
        self.apply_document_setting(DocumentSetting::EmulatedMedia(overrides))
    }

    pub(crate) fn set_idle_override(
        &mut self,
        idle_override: Option<crate::protocol_types::EmulatedIdleOverride>,
    ) -> anyhow::Result<()> {
        self.apply_document_setting(DocumentSetting::IdleOverride(idle_override))
    }

    pub(crate) fn set_viewport_surface(
        &mut self,
        viewport_surface: Option<crate::protocol_types::ViewportSurface>,
    ) -> anyhow::Result<()> {
        self.apply_document_setting(DocumentSetting::ViewportSurface(viewport_surface))
    }

    pub(crate) fn set_navigator_overrides(
        &mut self,
        overrides: &moli_page_types::NavigatorOverrides,
    ) -> anyhow::Result<()> {
        self.apply_document_setting(DocumentSetting::NavigatorOverrides(overrides))
    }

    pub(crate) fn set_document_activity(
        &mut self,
        activity: moli_page_types::DocumentActivity,
    ) -> anyhow::Result<()> {
        self.apply_document_setting(DocumentSetting::DocumentActivity(activity))
    }
}
