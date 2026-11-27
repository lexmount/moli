use std::{fs::File, io::Write, path::Path};

use serde::{Deserialize, Serialize};
use url::Url;

pub const HOME: &str = "about:blank";
pub const MAX_TABS: usize = 24;
pub const MIN_ZOOM: f64 = 0.5;
pub const MAX_ZOOM: f64 = 3.0;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Visit {
    pub url: String,
    pub title: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SavedTab {
    pub url: String,
    pub zoom: f64,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Session {
    pub selected: usize,
    pub tabs: Vec<SavedTab>,
    pub history: Vec<Visit>,
    pub bookmarks: Vec<Visit>,
}

impl Session {
    pub fn load(path: &Path) -> Self {
        File::open(path)
            .ok()
            .and_then(|file| serde_json::from_reader(file).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("missing session directory"))?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(&mut temp, self)?;
        temp.flush()?;
        temp.persist(path)?;
        Ok(())
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TabState {
    pub id: u64,
    pub url: String,
    pub title: String,
    pub pending_url: Option<String>,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub zoom: f64,
    pub editor: Option<EditorState>,
}

impl TabState {
    pub fn new(id: u64, zoom: f64) -> Self {
        Self {
            id,
            url: HOME.to_owned(),
            title: String::new(),
            pending_url: None,
            can_go_back: false,
            can_go_forward: false,
            loading: false,
            error: None,
            zoom,
            editor: None,
        }
    }
}

#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorState {
    pub token: u64,
    pub text: String,
    pub text_start: usize,
    pub start: usize,
    pub end: usize,
    pub password: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadState {
    pub id: u64,
    pub name: String,
    pub source: String,
    pub received: u64,
    pub total: Option<u64>,
    pub status: &'static str,
    pub path: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prompt {
    pub id: u64,
    pub kind: String,
    pub source: String,
    pub message: String,
    pub value: String,
    pub multiple: bool,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserState {
    pub revision: u64,
    pub ready: bool,
    pub error: Option<String>,
    pub selected_tab: u64,
    pub tabs: Vec<TabState>,
    pub history: Vec<Visit>,
    pub bookmarks: Vec<Visit>,
    pub downloads: Vec<DownloadState>,
    pub prompt: Option<Prompt>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
    pub dpr: f64,
}

impl Geometry {
    pub fn available(self) -> bool {
        self.width > 0 && self.height > 0 && self.dpr > 0.0
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TouchPhase {
    Down,
    Move,
    Up,
    Cancel,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScrollPhase {
    Begin,
    Move,
    End,
    Cancel,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum Edit {
    Insert { text: String },
    Compose { text: String, start: i32, end: i32 },
    Finish,
    Delete { count: usize, left: bool },
    Submit,
}

#[derive(Deserialize)]
pub struct PickedFile {
    pub name: String,
    pub path: String,
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Command {
    NewTab {
        url: Option<String>,
    },
    SelectTab {
        id: u64,
    },
    CloseTab {
        id: u64,
    },
    Open {
        tab: u64,
        input: String,
    },
    Home {
        tab: u64,
    },
    Back {
        tab: u64,
    },
    Forward {
        tab: u64,
    },
    Reload {
        tab: u64,
    },
    Stop {
        tab: u64,
    },
    Visible {
        visible: bool,
    },
    Resize {
        width: u32,
        height: u32,
        dpr: f64,
    },
    Zoom {
        tab: u64,
        zoom: f64,
    },
    Touch {
        tab: u64,
        phase: TouchPhase,
        x: f64,
        y: f64,
    },
    Scroll {
        tab: u64,
        phase: ScrollPhase,
        x: f64,
        y: f64,
        dx: f64,
        dy: f64,
    },
    Edit {
        tab: u64,
        token: u64,
        #[serde(flatten)]
        edit: Edit,
    },
    Bookmark {
        tab: u64,
    },
    ClearHistory,
    ClearDownloads,
    CancelDownload {
        id: u64,
    },
    RemoveDownload {
        id: u64,
    },
    Respond {
        id: u64,
        accepted: bool,
        value: String,
        files: Vec<PickedFile>,
    },
    #[serde(skip_deserializing)]
    Shutdown,
}

pub fn address_target(input: &str) -> Option<String> {
    let input = input.trim();
    if input.is_empty() {
        return Some(HOME.to_owned());
    }
    if let Ok(url) = Url::parse(input) {
        if matches!(url.scheme(), "http" | "https") {
            return url.host().is_some().then(|| url.to_string());
        }
        if url.as_str() == HOME {
            return Some(HOME.to_owned());
        }
        if input.contains("://") || matches!(url.scheme(), "javascript" | "data" | "file") {
            return None;
        }
    }
    if !input.contains(char::is_whitespace) {
        let authority = input.split('/').next().unwrap_or(input);
        let local = authority == "localhost"
            || authority.starts_with("localhost:")
            || authority.starts_with("127.")
            || authority.starts_with("[::1]")
            || authority.starts_with("10.0.2.2");
        if (local || authority.contains('.'))
            && let Ok(url) = Url::parse(&format!(
                "{}://{input}",
                if local { "http" } else { "https" }
            ))
            && url.host().is_some()
        {
            return Some(url.to_string());
        }
    }
    let mut search = Url::parse("https://www.baidu.com/s").expect("search URL");
    search.query_pairs_mut().append_pair("wd", input);
    Some(search.to_string())
}

pub fn web_url(url: &str) -> bool {
    Url::parse(url)
        .is_ok_and(|parsed| matches!(parsed.scheme(), "http" | "https") && parsed.has_host())
}
