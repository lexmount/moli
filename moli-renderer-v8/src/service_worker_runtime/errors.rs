#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerRegistrationErrorKind {
    Abort,
    Install,
    InvalidState,
    Network,
    NotFound,
    ScriptEvaluateFailed,
    Security,
    Type,
    Unknown,
}

impl ServiceWorkerRegistrationErrorKind {
    pub(crate) fn dom_exception_name(self) -> &'static str {
        match self {
            Self::Abort | Self::Install | Self::ScriptEvaluateFailed => "AbortError",
            Self::Network => "NetworkError",
            Self::InvalidState => "InvalidStateError",
            Self::NotFound => "NotFoundError",
            Self::Security => "SecurityError",
            Self::Type => unreachable!("type errors are not DOMExceptions"),
            Self::Unknown => "UnknownError",
        }
    }

    pub(crate) fn rejects_as_type_error_for_update(self) -> bool {
        matches!(
            self,
            Self::Network | Self::NotFound | Self::ScriptEvaluateFailed | Self::Type
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ServiceWorkerRegistrationError {
    pub(crate) kind: ServiceWorkerRegistrationErrorKind,
    pub(crate) message: String,
}

impl ServiceWorkerRegistrationError {
    pub(crate) fn new(
        kind: ServiceWorkerRegistrationErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub(crate) fn abort(message: impl Into<String>) -> Self {
        Self::new(ServiceWorkerRegistrationErrorKind::Abort, message)
    }

    pub(crate) fn install(message: impl Into<String>) -> Self {
        Self::new(ServiceWorkerRegistrationErrorKind::Install, message)
    }

    pub(crate) fn network(message: impl Into<String>) -> Self {
        Self::new(ServiceWorkerRegistrationErrorKind::Network, message)
    }

    pub(crate) fn type_error(message: impl Into<String>) -> Self {
        Self::new(ServiceWorkerRegistrationErrorKind::Type, message)
    }

    pub(crate) fn unknown(message: impl Into<String>) -> Self {
        Self::new(ServiceWorkerRegistrationErrorKind::Unknown, message)
    }
}

impl std::ops::Deref for ServiceWorkerRegistrationError {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

impl Default for ServiceWorkerRegistrationError {
    fn default() -> Self {
        Self::unknown(String::new())
    }
}
