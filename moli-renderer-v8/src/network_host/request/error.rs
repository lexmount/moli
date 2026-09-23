use crate::network_host::url_helpers::ResolveContextUrlError;
use crate::webidl;
use std::fmt;

/// Argument conversion failures remain structured until the fetch binding
/// turns them into a JavaScript rejection.
#[derive(Debug)]
pub(crate) enum FetchArgumentError {
    WebIdl(webidl::WebIdlError),
    UnsupportedNoCorsMethod {
        method: String,
        interface: Option<&'static str>,
    },
    Url(ResolveContextUrlError),
}

impl FetchArgumentError {
    pub(super) fn throw(&self, scope: &mut v8::PinScope<'_, '_>) {
        match self {
            Self::WebIdl(error) => webidl::throw_error(scope, error),
            _ => webidl::throw_type_error(scope, &self.to_string()),
        }
    }
}

impl From<webidl::WebIdlError> for FetchArgumentError {
    fn from(error: webidl::WebIdlError) -> Self {
        Self::WebIdl(error)
    }
}

impl From<ResolveContextUrlError> for FetchArgumentError {
    fn from(error: ResolveContextUrlError) -> Self {
        Self::Url(error)
    }
}

impl fmt::Display for FetchArgumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WebIdl(error) => error.fmt(f),
            Self::UnsupportedNoCorsMethod { method, interface } => {
                write!(f, "Failed to execute 'fetch'")?;
                if let Some(interface) = interface {
                    write!(f, " on '{interface}'")?;
                }
                write!(f, ": method `{method}` is unsupported in no-cors mode.")
            }
            Self::Url(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for FetchArgumentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::WebIdl(error) => Some(error),
            Self::Url(error) => Some(error),
            Self::UnsupportedNoCorsMethod { .. } => None,
        }
    }
}
