mod layout;
mod queries;

pub(crate) use layout::{
    compute_mock_client_rect, compute_mock_intersection_client_rect,
    compute_mock_intersection_scrollport_client_rect, compute_mock_scroll_adjusted_client_rect,
};
pub(super) use queries::answer_queries;
