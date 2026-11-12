use super::PageScriptHost;
use crate::runtime::{RendererPendingDownloadActivation, RendererPendingFileChooserActivation};

impl PageScriptHost {
    pub(crate) fn record_pending_file_chooser_activation(
        &mut self,
        mut activation: RendererPendingFileChooserActivation,
    ) {
        if let Some((handle, _)) = activation.live_node_source() {
            let Some(backend_node_id) = self.renderer_backend_node_id_for_live_handle(handle)
            else {
                return;
            };
            activation.backend_node_id = backend_node_id;
            activation.node_id = Some(handle);
            assert!(
                self.backend_node_registry
                    .borrow_mut()
                    .retain_detached_resolution(backend_node_id),
                "file chooser backend node id must exist before it is exposed"
            );
        }
        let published = self.append_live_turn_owner_action(
            crate::runtime::RendererOwnerAction::FileChooser(activation.clone()),
        );
        if published {
            return;
        }
        #[cfg(test)]
        self.pending_file_chooser_activations.push(activation);
        #[cfg(not(test))]
        {
            let _ = activation;
            panic!("a production file chooser must have a concrete renderer output sink");
        }
    }

    #[cfg(test)]
    pub(crate) fn take_pending_file_chooser_activations(
        &mut self,
    ) -> Vec<RendererPendingFileChooserActivation> {
        std::mem::take(&mut self.pending_file_chooser_activations)
    }

    pub(crate) fn pending_file_chooser_activation_count(&self) -> usize {
        #[cfg(test)]
        {
            self.pending_file_chooser_activations.len()
        }
        #[cfg(not(test))]
        {
            0
        }
    }

    pub(crate) fn record_pending_download_activation(
        &mut self,
        activation: RendererPendingDownloadActivation,
    ) {
        let published = self.append_live_turn_owner_action(
            crate::runtime::RendererOwnerAction::Download(activation.clone()),
        );
        if published {
            return;
        }
        #[cfg(test)]
        self.pending_download_activations.push(activation);
        #[cfg(not(test))]
        {
            let _ = activation;
            panic!("a production download must have a concrete renderer output sink");
        }
    }

    #[cfg(test)]
    pub(crate) fn take_pending_download_activations(
        &mut self,
    ) -> Vec<RendererPendingDownloadActivation> {
        std::mem::take(&mut self.pending_download_activations)
    }

    pub(crate) fn pending_download_activation_count(&self) -> usize {
        #[cfg(test)]
        {
            self.pending_download_activations.len()
        }
        #[cfg(not(test))]
        {
            0
        }
    }
}
