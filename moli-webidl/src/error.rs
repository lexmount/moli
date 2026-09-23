use std::fmt;

/// User-facing location for a WebIDL conversion error.
///
/// `prefix` is usually the API name or dictionary name, such as
/// `RequestInit` or `IDBFactory.open`. `member` is either an argument label
/// (`Argument 1`) or a dictionary member name. Keeping the context structured
/// lets generated parsers produce consistent browser-like TypeError text.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Context {
    prefix: &'static str,
    member: &'static str,
}

impl Context {
    /// Builds context for a positional Web API argument.
    pub const fn argument(prefix: &'static str, index: usize) -> Self {
        Self {
            prefix,
            member: match index {
                1 => "Argument 1",
                2 => "Argument 2",
                3 => "Argument 3",
                _ => "Argument",
            },
        }
    }

    /// Builds context for a named dictionary member or object property.
    pub const fn member(prefix: &'static str, member: &'static str) -> Self {
        Self { prefix, member }
    }
}

/// Classifies WebIDL conversion failures before they are rendered as TypeError.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum WebIdlErrorKind {
    CannotConvert(&'static str),
    InvalidEnum(&'static str),
    InvalidByteString,
    MissingRequired,
    CustomMessage(&'static str),
    PendingException,
}

/// Error returned by argument, dictionary, and scalar conversion helpers.
///
/// Most renderer callbacks pass this to `throw_error`, which turns conversion
/// failures into JavaScript `TypeError`s. `PendingException` is special: it
/// means V8 already has the original JavaScript exception rethrown, so
/// `throw_error` must not replace it with a generic TypeError.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WebIdlError {
    context: Context,
    kind: WebIdlErrorKind,
}

impl WebIdlError {
    pub(crate) fn new(context: Context, kind: WebIdlErrorKind) -> Self {
        Self { context, kind }
    }

    pub fn missing_required(context: Context) -> Self {
        Self::new(context, WebIdlErrorKind::MissingRequired)
    }

    pub fn cannot_convert(context: Context, target_type: &'static str) -> Self {
        Self::new(context, WebIdlErrorKind::CannotConvert(target_type))
    }

    pub fn custom_message(message: &'static str) -> Self {
        Self::new(
            Context::argument("", 1),
            WebIdlErrorKind::CustomMessage(message),
        )
    }

    pub fn custom_message_text(&self) -> Option<&'static str> {
        match self.kind {
            WebIdlErrorKind::CustomMessage(message) => Some(message),
            _ => None,
        }
    }

    pub fn pending_exception(context: Context) -> Self {
        Self::new(context, WebIdlErrorKind::PendingException)
    }

    pub fn is_pending_exception(&self) -> bool {
        matches!(self.kind, WebIdlErrorKind::PendingException)
    }
}

impl fmt::Debug for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Context")
            .field("prefix", &self.prefix)
            .field("member", &self.member)
            .finish()
    }
}

impl fmt::Display for WebIdlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prefix = if self.context.prefix.is_empty() {
            String::new()
        } else {
            format!("{}: ", self.context.prefix)
        };
        match self.kind {
            WebIdlErrorKind::CannotConvert(kind) => {
                write!(
                    f,
                    "{prefix}{} can not be converted to {kind}",
                    self.context.member
                )
            }
            WebIdlErrorKind::InvalidEnum(kind) => {
                write!(
                    f,
                    "{prefix}{} is not a valid enum value of type {kind}",
                    self.context.member
                )
            }
            WebIdlErrorKind::InvalidByteString => {
                write!(
                    f,
                    "{prefix}{} is not a valid ByteString",
                    self.context.member
                )
            }
            WebIdlErrorKind::MissingRequired => {
                write!(f, "{prefix}{} is required", self.context.member)
            }
            WebIdlErrorKind::CustomMessage(message) => write!(f, "{message}"),
            WebIdlErrorKind::PendingException => Ok(()),
        }
    }
}

impl std::error::Error for WebIdlError {}

#[cfg(test)]
mod tests {
    use super::{Context, WebIdlError, WebIdlErrorKind};

    #[test]
    fn webidl_error_formats_prefix_and_context() {
        let error = WebIdlError::new(
            Context::member("RequestInit", "method"),
            WebIdlErrorKind::CannotConvert("DOMString"),
        );
        assert_eq!(
            error.to_string(),
            "RequestInit: method can not be converted to DOMString"
        );
    }

    #[test]
    fn webidl_error_formats_invalid_enum_context() {
        let error = WebIdlError::new(
            Context::member("WorkerOptions", "type"),
            WebIdlErrorKind::InvalidEnum("WorkerType"),
        );
        assert_eq!(
            error.to_string(),
            "WorkerOptions: type is not a valid enum value of type WorkerType"
        );
    }
}
