/* SPDX-License-Identifier: curl */
#ifndef CURLINC_MOLI_TIMING_H
#define CURLINC_MOLI_TIMING_H

#include "curl.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Private Moli extension. Times use the supplied process-lifetime monotonic
 * clock, never libcurl's duration origin. A zero transport time is unknown. */
typedef curl_off_t (*curl_moli_clock_callback)(void);

struct curl_moli_connection_timing {
  curl_off_t dns_start;
  curl_off_t dns_end;
  curl_off_t has_dns;
  curl_off_t connect_start;
  curl_off_t connect_end;
  curl_off_t secure_start;
};

CURL_EXTERN CURLcode curl_easy_moli_timing_init(
  CURL *easy, curl_moli_clock_callback clock,
  curl_off_t dns_start, curl_off_t dns_end, int has_dns);
CURL_EXTERN CURLcode curl_easy_moli_connection_timing(
  CURL *easy, struct curl_moli_connection_timing *timing);

#ifdef __cplusplus
}
#endif

#endif
