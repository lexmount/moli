/* SPDX-License-Identifier: curl */
#ifndef HEADER_CURL_MOLI_TIMING_H
#define HEADER_CURL_MOLI_TIMING_H

#include "urldata.h"

/* The optional clock has static lifetime and must not unwind into C. */
curl_off_t Curl_moli_now(struct Curl_easy *data);

#endif
