use super::*;
use moli_indexeddb::sqlite::{Query, service::InFlightData};

pub(in crate::context_bootstrap::indexed_db) struct SqlCursorQuery {
    pub(in crate::context_bootstrap::indexed_db) store: String,
    pub(in crate::context_bootstrap::indexed_db) query: Query,
}

/// A lazy cursor keeps its native row and reservation together until the row
/// is replaced or the cursor is collected. Pending results have another page.
pub(in crate::context_bootstrap::indexed_db) struct SqlCursorPage {
    pub(in crate::context_bootstrap::indexed_db) query: Rc<SqlCursorQuery>,
    entries: Rc<[CursorSnapshotEntry]>,
    permit: RefCell<InFlightData>,
}

pub(in crate::context_bootstrap::indexed_db) fn attach_indexed_db_sql_cursor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cursor: v8::Local<'s, v8::Object>,
    query: Rc<SqlCursorQuery>,
    permit: InFlightData,
) -> Option<()> {
    let state = cursor_native_state(scope, cursor)?;
    let mut state = state.borrow_mut();
    state.sql = Some(Rc::new(SqlCursorPage {
        entries: Rc::clone(&state.entries),
        query,
        permit: RefCell::new(permit),
    }));
    Some(())
}

pub(in crate::context_bootstrap::indexed_db) fn stage_indexed_db_sql_cursor_page<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    query: Rc<SqlCursorQuery>,
    entries: Vec<CursorSnapshotEntry>,
    permit: InFlightData,
) -> Option<()> {
    let id = indexed_db_typed_state_id(scope, request)?;
    let table = indexed_db_runtime_state_table_for_object(scope, request);
    table
        .borrow_mut()
        .requests
        .get_mut(&id)?
        .pending_cursor_page = Some(Rc::new(SqlCursorPage {
        query,
        entries: entries.into(),
        permit: RefCell::new(permit),
    }));
    Some(())
}

pub(in crate::context_bootstrap::indexed_db) fn publish_indexed_db_sql_cursor_page<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    cursor: v8::Local<'s, v8::Object>,
) {
    let Some(id) = indexed_db_typed_state_id(scope, request) else {
        return;
    };
    let table = indexed_db_runtime_state_table_for_object(scope, request);
    let page = table
        .borrow_mut()
        .requests
        .get_mut(&id)
        .and_then(|state| state.pending_cursor_page.take());
    if let Some(page) = page
        && let Some(state) = cursor_native_state(scope, cursor)
    {
        let mut state = state.borrow_mut();
        state.entries = Rc::clone(&page.entries);
        state.sql = (!page.entries.is_empty()).then_some(page);
    }
}

// Borrow only while adjusting the native counter. Host-object creation may
// enter V8 and reenter a cursor getter; no RefCell guard crosses that boundary.
impl moli_indexeddb::ReadBudget for &SqlCursorPage {
    fn charge(&mut self, bytes: u64) -> Result<(), IndexedDbError> {
        moli_indexeddb::ReadBudget::charge(&mut *self.permit.borrow_mut(), bytes)
    }
    fn refund(&mut self, bytes: u64) {
        moli_indexeddb::ReadBudget::refund(&mut *self.permit.borrow_mut(), bytes);
    }
}
