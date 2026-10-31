/* SPDX-License-Identifier: curl */
#ifndef CURLINC_MOLI_WEBSOCKET_H
#define CURLINC_MOLI_WEBSOCKET_H

#include "curl.h"
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Private native-owner extension. Callbacks run synchronously on the Multi
 * owner before TCP socket creation. acquire returns 1 to connect, 0 to wait,
 * or -1 to fail. Address octets are in network order; port is in host order.
 * Each connection filter has a unique attempt token while it exists.
 *
 * The Easy borrows this structure until cleanup or replacement. A completed
 * handshake must release the owner's leases. Filter destruction additionally
 * releases failed/losing address attempts. No callback state is retained by
 * pooled connections, and duplicating an Easy does not copy its gate. */
struct curl_moli_websocket_gate {
  uintptr_t userdata;
  int (*acquire)(uintptr_t userdata, uintptr_t attempt,
                 const unsigned char *address, unsigned int length,
                 unsigned short port, unsigned int scope);
  void (*release)(uintptr_t userdata, uintptr_t attempt);
};

CURL_EXTERN CURLcode curl_easy_moli_websocket_set_gate(
  CURL *easy, const struct curl_moli_websocket_gate *gate);
CURL_EXTERN CURLcode curl_easy_moli_websocket_resume(CURL *easy);

#ifdef __cplusplus
}
#endif

#endif
