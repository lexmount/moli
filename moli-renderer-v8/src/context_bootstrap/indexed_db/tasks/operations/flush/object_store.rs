use super::*;

mod read;
mod write;

pub(super) fn try_dispatch_object_store_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    operation: &operation::TransactionOperation<'s>,
) -> bool {
    write::try_dispatch_object_store_write_operation(scope, operation)
        || read::try_dispatch_object_store_read_operation(scope, operation)
}
