// Copyright 2026 the Moli authors. MIT license.

#include "v8-isolate.h"

extern "C" void v8__Isolate__LocaleConfigurationChangeNotification(
    v8::Isolate* isolate) {
  isolate->LocaleConfigurationChangeNotification();
}
