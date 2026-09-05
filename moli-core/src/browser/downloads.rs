/// Browser policy only: no frontend subscriptions or session attribution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadPolicy {
    pub behavior: DownloadBehavior,
    pub download_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DownloadBehavior {
    #[default]
    Default,
    Deny,
    Allow,
    AllowAndName,
}

impl DownloadBehavior {
    pub fn allows_download(self) -> bool {
        matches!(self, Self::Allow | Self::AllowAndName)
    }

    pub fn names_artifact_by_guid(self) -> bool {
        self == Self::AllowAndName
    }

    pub fn is_canceled_without_download(self) -> bool {
        matches!(self, Self::Default | Self::Deny)
    }
}
