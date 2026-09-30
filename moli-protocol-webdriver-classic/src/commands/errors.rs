use moli_protocol::automation::{DevToolsError, DevToolsErrorKind};

use crate::{ClassicError, ClassicErrorCode};

pub fn classic_error_from_devtools_error(error: DevToolsError) -> ClassicError {
    let code = match error.kind {
        DevToolsErrorKind::InvalidArgument | DevToolsErrorKind::UnableToSetFileInput => {
            ClassicErrorCode::InvalidArgument
        }
        DevToolsErrorKind::InvalidSelector => ClassicErrorCode::InvalidSelector,
        DevToolsErrorKind::NoSuchTarget => ClassicErrorCode::NoSuchWindow,
        DevToolsErrorKind::NoSuchSession => ClassicErrorCode::InvalidSessionId,
        DevToolsErrorKind::NoSuchAlert => ClassicErrorCode::NoSuchAlert,
        DevToolsErrorKind::NoSuchHandle | DevToolsErrorKind::NoSuchNode => {
            ClassicErrorCode::StaleElementReference
        }
        DevToolsErrorKind::Unsupported => ClassicErrorCode::UnsupportedOperation,
        DevToolsErrorKind::NoSuchHistoryEntry
        | DevToolsErrorKind::NoSuchNetworkCollector
        | DevToolsErrorKind::NoSuchNetworkData
        | DevToolsErrorKind::NoSuchRequest
        | DevToolsErrorKind::NoSuchScript
        | DevToolsErrorKind::NavigationChangingDocument
        | DevToolsErrorKind::UnableToCaptureScreen
        | DevToolsErrorKind::Internal => ClassicErrorCode::UnknownError,
        DevToolsErrorKind::Timeout => ClassicErrorCode::Timeout,
    };
    ClassicError::new(code, error.message)
}
