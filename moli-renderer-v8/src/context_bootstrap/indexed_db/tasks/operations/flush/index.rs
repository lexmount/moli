use super::*;

mod read;

pub(super) fn try_dispatch_index_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    operation: &operation::TransactionOperation<'s>,
) -> bool {
    read::try_dispatch_index_read_operation(scope, operation)
}
