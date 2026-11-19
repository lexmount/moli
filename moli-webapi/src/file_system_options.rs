use moli_webidl as webidl;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, webidl::WebIdlEnum)]
#[webidl(name = "FileSystemWritableFileStreamMode", rename_all = "lowercase")]
pub enum FileSystemWritableMode {
    Exclusive,
    #[default]
    Siloed,
}

impl FileSystemWritableMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exclusive => "exclusive",
            Self::Siloed => "siloed",
        }
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "FileSystemCreateWritableOptions")]
pub struct FileSystemCreateWritableOptions {
    #[webidl(name = "keepExistingData", default = false)]
    pub keep_existing_data: bool,
    #[webidl(
        converter = "enum",
        default = FileSystemWritableMode::Siloed
    )]
    pub mode: FileSystemWritableMode,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, webidl::WebIdlEnum)]
#[webidl(name = "FileSystemSyncAccessHandleMode", rename_all = "kebab-case")]
pub enum FileSystemSyncAccessHandleMode {
    InPlace,
    ReadOnly,
    #[default]
    Readwrite,
    ReadwriteUnsafe,
}

impl FileSystemSyncAccessHandleMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InPlace => "in-place",
            Self::ReadOnly => "read-only",
            Self::Readwrite => "readwrite",
            Self::ReadwriteUnsafe => "readwrite-unsafe",
        }
    }
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "FileSystemCreateSyncAccessHandleOptions")]
pub struct FileSystemCreateSyncAccessHandleOptions {
    #[webidl(
        converter = "enum",
        default = FileSystemSyncAccessHandleMode::Readwrite
    )]
    pub mode: FileSystemSyncAccessHandleMode,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, webidl::WebIdlEnum)]
#[webidl(name = "FileSystemPermissionMode", rename_all = "lowercase")]
pub enum FileSystemPermissionMode {
    #[default]
    Read,
    Readwrite,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "FileSystemHandlePermissionDescriptor")]
pub struct FileSystemHandlePermissionDescriptor {
    #[webidl(converter = "enum", default = FileSystemPermissionMode::Read)]
    pub mode: FileSystemPermissionMode,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "FileSystemReadWriteOptions")]
pub struct FileSystemReadWriteOptions {
    #[webidl(converter = "enforce_range_unsigned_long_long")]
    pub at: Option<u64>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FileSystemSyncAccessHandle.truncate")]
pub struct FileSystemSyncAccessHandleTruncateArgs {
    #[webidl(required, converter = "enforce_range_unsigned_long_long")]
    pub size: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, webidl::WebIdlEnum)]
#[webidl(name = "WriteCommandType", rename_all = "lowercase")]
pub enum FileSystemWriteCommandType {
    Write,
    Seek,
    Truncate,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "WriteParams")]
pub struct FileSystemWriteParams<'s> {
    #[webidl(name = "type", required, converter = "enum")]
    pub command_type: FileSystemWriteCommandType,
    #[webidl(nullable)]
    pub size: Option<u64>,
    #[webidl(nullable)]
    pub position: Option<u64>,
    #[webidl(converter = "raw")]
    pub data: Option<v8::Local<'s, v8::Value>>,
}
