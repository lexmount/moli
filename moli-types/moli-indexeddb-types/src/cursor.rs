use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
pub enum CursorDirection {
    Next,
    NextUnique,
    Prev,
    PrevUnique,
}

impl CursorDirection {
    pub const fn default_next() -> Self {
        Self::Next
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }

    pub fn as_str(self) -> &'static str {
        self.into()
    }

    pub const fn is_reverse(self) -> bool {
        matches!(self, Self::Prev | Self::PrevUnique)
    }

    pub const fn is_unique(self) -> bool {
        matches!(self, Self::NextUnique | Self::PrevUnique)
    }
}
