//! Shared table roles used by construction and invariant validation.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TableRole {
    Root,
    Caption,
    RowGroup,
    ColumnGroup,
    Column,
    Row,
    Cell,
}

impl TableRole {
    pub(crate) const fn requires_parent(self) -> bool {
        !matches!(self, Self::Root)
    }

    pub(crate) const fn is_direct_table_child(self) -> bool {
        matches!(
            self,
            Self::Caption | Self::RowGroup | Self::ColumnGroup | Self::Column
        )
    }

    pub(crate) const fn debug_name(self) -> &'static str {
        match self {
            Self::Root => "table-root",
            Self::Caption => "table-caption",
            Self::RowGroup => "table-row-group",
            Self::ColumnGroup => "table-column-group",
            Self::Column => "table-column",
            Self::Row => "table-row",
            Self::Cell => "table-cell",
        }
    }
    pub(crate) const fn from_display(display: crate::LayoutDisplay) -> Option<Self> {
        use crate::LayoutDisplay as Display;
        match display {
            Display::Table | Display::InlineTable => Some(Self::Root),
            Display::TableCaption => Some(Self::Caption),
            Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup => {
                Some(Self::RowGroup)
            }
            Display::TableColumnGroup => Some(Self::ColumnGroup),
            Display::TableColumn => Some(Self::Column),
            Display::TableRow => Some(Self::Row),
            Display::TableCell => Some(Self::Cell),
            Display::None
            | Display::Contents
            | Display::Block
            | Display::FlowRoot
            | Display::Inline
            | Display::InlineBlock
            | Display::Flex
            | Display::InlineFlex
            | Display::Grid
            | Display::InlineGrid
            | Display::BlockListItem
            | Display::InlineListItem => None,
        }
    }
}
