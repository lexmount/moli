use super::PreparedScript;
use crate::runtime::{
    RendererScriptExecutionMemoryDiagnostics, RendererScriptSourceMemoryDiagnostics,
};
use crate::types::{ScriptKind, ScriptMode, ScriptSourceKind};

#[derive(Default)]
pub(super) struct ScriptExecutionMemoryCounters {
    execution_count: usize,
    total_source_bytes: usize,
    inline_source_bytes: usize,
    external_source_bytes: usize,
    classic_source_bytes: usize,
    module_source_bytes: usize,
    import_map_source_bytes: usize,
    data_block_source_bytes: usize,
    inline_execution_count: usize,
    external_execution_count: usize,
    classic_execution_count: usize,
    module_execution_count: usize,
    import_map_execution_count: usize,
    data_block_execution_count: usize,
    largest_sources: Vec<ScriptSourceMemorySample>,
}

#[derive(Clone)]
struct ScriptSourceMemorySample {
    url: String,
    source_bytes: usize,
    kind: ScriptKind,
    mode: ScriptMode,
    source_kind: ScriptSourceKind,
}

impl ScriptExecutionMemoryCounters {
    pub(super) fn record(&mut self, script: &PreparedScript, source_len: usize) {
        self.execution_count += 1;
        self.total_source_bytes = self.total_source_bytes.saturating_add(source_len);
        match script.source_kind {
            ScriptSourceKind::Inline => {
                self.inline_execution_count += 1;
                self.inline_source_bytes = self.inline_source_bytes.saturating_add(source_len);
            }
            ScriptSourceKind::External => {
                self.external_execution_count += 1;
                self.external_source_bytes = self.external_source_bytes.saturating_add(source_len);
            }
        }
        match script.kind {
            ScriptKind::Classic => {
                self.classic_execution_count += 1;
                self.classic_source_bytes = self.classic_source_bytes.saturating_add(source_len);
            }
            ScriptKind::Module => {
                self.module_execution_count += 1;
                self.module_source_bytes = self.module_source_bytes.saturating_add(source_len);
            }
            ScriptKind::ImportMap => {
                self.import_map_execution_count += 1;
                self.import_map_source_bytes =
                    self.import_map_source_bytes.saturating_add(source_len);
            }
            ScriptKind::DataBlock => {
                self.data_block_execution_count += 1;
                self.data_block_source_bytes =
                    self.data_block_source_bytes.saturating_add(source_len);
            }
        }
        self.largest_sources.push(ScriptSourceMemorySample {
            url: script.url.as_str().to_owned(),
            source_bytes: source_len,
            kind: script.kind,
            mode: script.mode,
            source_kind: script.source_kind,
        });
        self.largest_sources.sort_by(|left, right| {
            right
                .source_bytes
                .cmp(&left.source_bytes)
                .then_with(|| left.url.cmp(&right.url))
        });
        self.largest_sources.truncate(12);
    }

    pub(super) fn to_diagnostics(&self) -> RendererScriptExecutionMemoryDiagnostics {
        RendererScriptExecutionMemoryDiagnostics {
            execution_count: self.execution_count,
            total_source_bytes: self.total_source_bytes,
            inline_source_bytes: self.inline_source_bytes,
            external_source_bytes: self.external_source_bytes,
            classic_source_bytes: self.classic_source_bytes,
            module_source_bytes: self.module_source_bytes,
            import_map_source_bytes: self.import_map_source_bytes,
            data_block_source_bytes: self.data_block_source_bytes,
            inline_execution_count: self.inline_execution_count,
            external_execution_count: self.external_execution_count,
            classic_execution_count: self.classic_execution_count,
            module_execution_count: self.module_execution_count,
            import_map_execution_count: self.import_map_execution_count,
            data_block_execution_count: self.data_block_execution_count,
            largest_sources: self
                .largest_sources
                .iter()
                .map(|sample| RendererScriptSourceMemoryDiagnostics {
                    url: sample.url.clone(),
                    source_bytes: sample.source_bytes,
                    kind: format!("{:?}", sample.kind),
                    mode: format!("{:?}", sample.mode),
                    source_kind: format!("{:?}", sample.source_kind),
                })
                .collect(),
        }
    }
}
