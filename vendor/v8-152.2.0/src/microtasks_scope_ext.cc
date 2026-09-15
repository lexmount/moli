#include "support.h"
#include "v8-microtask-queue.h"

using namespace support;

static_assert(sizeof(v8::MicrotasksScope) == sizeof(uintptr_t) * 3);
static_assert(alignof(v8::MicrotasksScope) == alignof(uintptr_t));

extern "C" {
void v8__MicrotasksScope__CONSTRUCT(uninit_t<v8::MicrotasksScope>* storage,
                                  const v8::Context& context,
                                  v8::MicrotasksScope::Type type) {
  construct_in_place<v8::MicrotasksScope>(storage, ptr_to_local(&context), type);
}

void v8__MicrotasksScope__DESTRUCT(v8::MicrotasksScope* scope) {
  scope->~MicrotasksScope();
}
}
