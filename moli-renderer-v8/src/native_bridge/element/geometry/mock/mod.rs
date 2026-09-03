mod layout;
mod queries;

pub(in crate::native_bridge::element) use layout::element_has_hidden_attribute;
pub(crate) use layout::{
    compute_mock_client_rect, compute_mock_intersection_client_rect,
    compute_mock_intersection_scrollport_client_rect, compute_mock_scroll_adjusted_client_rect,
};
pub(super) use queries::answer_queries;
