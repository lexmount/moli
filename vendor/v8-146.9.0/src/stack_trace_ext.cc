#include "support.h"
#include "v8.h"

#include <array>

using namespace support;

extern "C" {

const v8::Context* v8__StackTrace__CurrentScriptContext(
    v8::Isolate* isolate) {
  std::array<v8::StackTrace::ScriptIdAndContext, 1> frame_data;
  auto frames = v8::StackTrace::CurrentScriptIdsAndContexts(isolate,
                                                             frame_data);
  if (frames.empty()) {
    return nullptr;
  }
  return local_to_ptr(frames[0].context);
}

}  // extern "C"
