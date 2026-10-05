use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{Value, json};

type ToolId = (String, String);

#[derive(Default)]
pub(super) struct Catalog {
    tools: BTreeMap<ToolId, SiteTool>,
    aliases: HashMap<ToolId, String>,
    used_names: HashSet<String>,
}

#[derive(Clone)]
pub(super) struct SiteTool {
    pub(super) frame_id: String,
    pub(super) name: String,
    pub(super) backend_node_id: Option<u64>,
    pub(super) autosubmit: bool,
    pub(super) mcp: Value,
}

impl Catalog {
    pub(super) fn add(&mut self, raw: &Value, target_id: &str) -> bool {
        let (Some(name), Some(frame_id)) = (raw["name"].as_str(), raw["frameId"].as_str()) else {
            return false;
        };
        let id = (frame_id.to_owned(), name.to_owned());
        let alias = self.aliases.entry(id.clone()).or_insert_with(|| {
            let stem: String = name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || "_.-".contains(c) {
                        c
                    } else {
                        '_'
                    }
                })
                .take(110)
                .collect();
            let stem = if stem.is_empty() {
                "tool".to_owned()
            } else {
                stem
            };
            let mut alias = stem.clone();
            let mut suffix = 2;
            while !self.used_names.insert(alias.clone()) {
                alias = format!("{stem}__{suffix}");
                suffix += 1;
            }
            alias
        });
        let schema = raw
            .get("inputSchema")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| json!({"type":"object", "properties":{}}));
        let mut mcp = json!({
            "name": alias,
            "description": raw["description"].as_str().unwrap_or_default(),
            "inputSchema": schema,
            "_meta": {
                "moli/webmcp": {
                    "targetId": target_id, "frameId": frame_id, "name": name,
                    "backendNodeId": raw.get("backendNodeId"),
                    "annotations": raw.get("annotations")
                }
            }
        });
        if let Some(read_only) = raw["annotations"]["readOnly"].as_bool() {
            // Consequential and untrustedContent have different semantics from
            // MCP's destructiveHint and openWorldHint; preserve them in _meta.
            mcp["annotations"] = json!({"readOnlyHint": read_only});
        }
        let changed = self.tools.get(&id).is_none_or(|tool| tool.mcp != mcp);
        self.tools.insert(
            id,
            SiteTool {
                frame_id: frame_id.to_owned(),
                name: name.to_owned(),
                backend_node_id: raw["backendNodeId"].as_u64(),
                autosubmit: raw["annotations"]["autosubmit"].as_bool().unwrap_or(false),
                mcp,
            },
        );
        changed
    }

    pub(super) fn remove(&mut self, raw: &Value) -> bool {
        let (Some(name), Some(frame_id)) = (raw["name"].as_str(), raw["frameId"].as_str()) else {
            return false;
        };
        self.tools
            .remove(&(frame_id.to_owned(), name.to_owned()))
            .is_some()
    }

    pub(super) fn remove_frame(&mut self, frame_id: &str) -> bool {
        let before = self.tools.len();
        self.tools.retain(|(frame, _), _| frame != frame_id);
        before != self.tools.len()
    }

    pub(super) fn clear(&mut self) -> bool {
        let changed = !self.tools.is_empty();
        self.tools.clear();
        changed
    }

    pub(super) fn list(&self) -> Vec<Value> {
        let mut tools: Vec<_> = self.tools.values().map(|tool| tool.mcp.clone()).collect();
        tools.sort_unstable_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        tools
    }

    pub(super) fn find(&self, alias: &str) -> Option<SiteTool> {
        self.tools
            .values()
            .find(|tool| tool.mcp["name"] == alias)
            .cloned()
    }
}
