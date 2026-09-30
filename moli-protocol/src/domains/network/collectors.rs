use std::collections::{HashMap, HashSet};

use crate::automation::{
    AutomationResult, DevToolsAddNetworkDataCollectorCommand,
    DevToolsAddNetworkDataCollectorResult, DevToolsError, DevToolsErrorKind,
    DevToolsNetworkDataCollectorId, DevToolsNetworkDataType,
};
use crate::conn::CapturedBody;

const MAX_TOTAL_COLLECTED_SIZE: u64 = 200_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
struct NetworkDataCollectorConfig {
    data_types: HashSet<DevToolsNetworkDataType>,
    max_encoded_data_size: u64,
    target_ids: HashSet<String>,
    browser_context_ids: HashSet<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct NetworkDataCollectorStore {
    collectors: HashMap<String, NetworkDataCollectorConfig>,
    disowned: HashSet<(String, DevToolsNetworkDataType, String)>,
    collected_bodies: HashMap<(String, DevToolsNetworkDataType), CollectedNetworkDataBody>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CollectedNetworkDataBody {
    body: CapturedBody,
    collector_ids: HashSet<String>,
}

impl NetworkDataCollectorStore {
    pub(crate) fn add_collector(
        &mut self,
        command: DevToolsAddNetworkDataCollectorCommand,
    ) -> Result<AutomationResult, DevToolsError> {
        if command.max_encoded_data_size < 1
            || command.max_encoded_data_size > MAX_TOTAL_COLLECTED_SIZE
        {
            return Err(DevToolsError::new(
                DevToolsErrorKind::InvalidArgument,
                format!("Max encoded data size should be between 1 and {MAX_TOTAL_COLLECTED_SIZE}"),
            ));
        }
        let collector_id = command.collector_id.into_string();
        self.collectors.insert(
            collector_id.clone(),
            NetworkDataCollectorConfig {
                data_types: command.data_types.into_iter().collect(),
                max_encoded_data_size: command.max_encoded_data_size,
                target_ids: command
                    .target_ids
                    .into_iter()
                    .map(|target_id| target_id.into_string())
                    .collect(),
                browser_context_ids: command
                    .browser_context_ids
                    .into_iter()
                    .map(|browser_context_id| browser_context_id.into_string())
                    .collect(),
            },
        );
        Ok(AutomationResult::AddNetworkDataCollector(
            DevToolsAddNetworkDataCollectorResult {
                collector_id: DevToolsNetworkDataCollectorId::from(collector_id),
            },
        ))
    }

    pub(crate) fn remove_collector(
        &mut self,
        collector_id: &DevToolsNetworkDataCollectorId,
    ) -> Result<(), DevToolsError> {
        let collector_id = collector_id.as_str();
        if self.collectors.remove(collector_id).is_none() {
            return Err(no_such_network_collector());
        }
        self.disowned
            .retain(|(_, _, disowned_collector)| disowned_collector != collector_id);
        self.collected_bodies.retain(|_, body| {
            body.remove_collector(collector_id);
            !body.collector_ids.is_empty()
        });
        Ok(())
    }

    pub(crate) fn ensure_collector_exists(
        &self,
        collector_id: &DevToolsNetworkDataCollectorId,
    ) -> Result<(), DevToolsError> {
        if self.collectors.contains_key(collector_id.as_str()) {
            Ok(())
        } else {
            Err(no_such_network_collector())
        }
    }

    pub(crate) fn collector_ids_for_body(
        &self,
        data_type: DevToolsNetworkDataType,
        encoded_data_size: usize,
        target_id: Option<&str>,
        browser_context_id: Option<&str>,
    ) -> HashSet<String> {
        self.collectors
            .iter()
            .filter(|(_, collector)| {
                collector.collects_body(data_type, encoded_data_size, target_id, browser_context_id)
            })
            .map(|(collector_id, _)| collector_id.clone())
            .collect()
    }

    pub(crate) fn has_collector_for_data_type(&self, data_type: DevToolsNetworkDataType) -> bool {
        self.collectors
            .values()
            .any(|collector| collector.collects_data_type(data_type))
    }

    pub(crate) fn body_is_collected(
        &self,
        request_id: &str,
        data_type: DevToolsNetworkDataType,
        collector_id: &DevToolsNetworkDataCollectorId,
        body_was_collected_by_collector: bool,
    ) -> Result<bool, DevToolsError> {
        let collector_id_string = collector_id.as_str();
        let Some(collector) = self.collectors.get(collector_id_string) else {
            return Err(no_such_network_collector());
        };
        if !collector.collects_data_type(data_type) {
            return Ok(false);
        }
        if self.disowned.contains(&(
            request_id.to_owned(),
            data_type,
            collector_id_string.to_owned(),
        )) {
            return Ok(false);
        }
        Ok(body_was_collected_by_collector)
    }

    pub(crate) fn body_has_owned_collector(
        &self,
        request_id: &str,
        data_type: DevToolsNetworkDataType,
        collector_ids: &HashSet<String>,
    ) -> bool {
        collector_ids.iter().any(|collector_id| {
            self.collectors
                .get(collector_id)
                .is_some_and(|collector| collector.collects_data_type(data_type))
                && !self.disowned.contains(&(
                    request_id.to_owned(),
                    data_type,
                    collector_id.clone(),
                ))
        })
    }

    pub(crate) fn disown_data(
        &mut self,
        request_id: &str,
        data_type: DevToolsNetworkDataType,
        collector_id: &DevToolsNetworkDataCollectorId,
    ) -> Result<(), DevToolsError> {
        self.ensure_collector_exists(collector_id)?;
        self.disowned.insert((
            request_id.to_owned(),
            data_type,
            collector_id.as_str().to_owned(),
        ));
        Ok(())
    }

    pub(crate) fn record_collected_body(
        &mut self,
        request_id: String,
        data_type: DevToolsNetworkDataType,
        body: CapturedBody,
        collector_ids: impl IntoIterator<Item = String>,
        _collection_was_gated: bool,
    ) {
        let collector_ids = collector_ids
            .into_iter()
            .filter(|collector_id| {
                self.collectors
                    .get(collector_id)
                    .is_some_and(|collector| collector.collects_data_type(data_type))
            })
            .collect::<HashSet<_>>();
        if collector_ids.is_empty() {
            return;
        }
        self.collected_bodies.insert(
            (request_id, data_type),
            CollectedNetworkDataBody::new(body, collector_ids),
        );
    }

    pub(crate) fn collected_body(
        &self,
        request_id: &str,
        data_type: DevToolsNetworkDataType,
    ) -> Option<&CollectedNetworkDataBody> {
        self.collected_bodies
            .get(&(request_id.to_owned(), data_type))
    }
}

impl NetworkDataCollectorConfig {
    fn collects_data_type(&self, data_type: DevToolsNetworkDataType) -> bool {
        self.data_types.contains(&data_type)
    }

    fn collects_body(
        &self,
        data_type: DevToolsNetworkDataType,
        encoded_data_size: usize,
        target_id: Option<&str>,
        browser_context_id: Option<&str>,
    ) -> bool {
        self.collects_data_type(data_type)
            && self.scope_matches(target_id, browser_context_id)
            && u64::try_from(encoded_data_size).is_ok_and(|size| size <= self.max_encoded_data_size)
    }

    fn scope_matches(&self, target_id: Option<&str>, browser_context_id: Option<&str>) -> bool {
        if !self.target_ids.is_empty() {
            return target_id.is_some_and(|target_id| self.target_ids.contains(target_id));
        }
        if !self.browser_context_ids.is_empty() {
            return browser_context_id.is_some_and(|browser_context_id| {
                self.browser_context_ids.contains(browser_context_id)
            });
        }
        true
    }
}

impl CollectedNetworkDataBody {
    fn new(body: CapturedBody, collector_ids: HashSet<String>) -> Self {
        Self {
            body,
            collector_ids,
        }
    }

    pub(crate) fn body_bytes_limited(&self, limit: usize) -> anyhow::Result<Vec<u8>> {
        self.body.materialize_bytes_limited(limit)
    }

    pub(crate) fn was_collected_by(&self, collector_id: &str) -> bool {
        self.collector_ids.contains(collector_id)
    }

    pub(crate) fn collector_ids(&self) -> &HashSet<String> {
        &self.collector_ids
    }

    fn remove_collector(&mut self, collector_id: &str) {
        self.collector_ids.remove(collector_id);
    }
}

fn no_such_network_collector() -> DevToolsError {
    DevToolsError::new(
        DevToolsErrorKind::NoSuchNetworkCollector,
        "no such network collector",
    )
}
