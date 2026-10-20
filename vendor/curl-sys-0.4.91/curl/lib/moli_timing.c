/* SPDX-License-Identifier: curl */
#include "curl_setup.h"
#include "moli_timing.h"

curl_off_t Curl_moli_now(struct Curl_easy *data)
{
  return data->set.moli_clock ? data->set.moli_clock() : 0;
}

CURLcode curl_easy_moli_timing_init(CURL *easy, curl_moli_clock_callback clock,
                                   curl_off_t dns_start, curl_off_t dns_end,
                                   int has_dns)
{
  struct Curl_easy *data = easy;
  if(!data)
    return CURLE_BAD_FUNCTION_ARGUMENT;
  data->set.moli_clock = clock;
  data->set.moli_dns_start = dns_start;
  data->set.moli_dns_end = dns_end;
  data->set.moli_has_dns = !!has_dns;
  memset(&data->progress.moli_connection, 0,
         sizeof(data->progress.moli_connection));
  return CURLE_OK;
}

CURLcode curl_easy_moli_connection_timing(
  CURL *easy, struct curl_moli_connection_timing *timing)
{
  struct Curl_easy *data = easy;
  if(!data || !timing)
    return CURLE_BAD_FUNCTION_ARGUMENT;
  if(!data->progress.moli_connection.connect_start ||
     !data->progress.moli_connection.connect_end)
    return CURLE_AGAIN;
  *timing = data->progress.moli_connection;
  return CURLE_OK;
}
