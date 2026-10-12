use std::{ops::Deref, sync::Arc};

use super::{Attribute, SourceLocation};

/// Attributes from one parser token and the location of their change steps.
/// Provenance travels with the token without enlarging ordinary DOM attributes.
#[derive(Clone, Debug, Default)]
pub struct ParserAttributes {
    values: Vec<Attribute>,
    source_location: Option<Arc<SourceLocation>>,
}

impl ParserAttributes {
    pub fn new(values: Vec<Attribute>, source_location: Option<Arc<SourceLocation>>) -> Self {
        Self {
            values,
            source_location,
        }
    }

    pub fn source_location(&self) -> Option<&Arc<SourceLocation>> {
        self.source_location.as_ref()
    }

    pub fn into_parts(self) -> (Vec<Attribute>, Option<Arc<SourceLocation>>) {
        (self.values, self.source_location)
    }
}

impl Deref for ParserAttributes {
    type Target = [Attribute];

    fn deref(&self) -> &Self::Target {
        &self.values
    }
}

impl From<Vec<Attribute>> for ParserAttributes {
    fn from(values: Vec<Attribute>) -> Self {
        Self::new(values, None)
    }
}
