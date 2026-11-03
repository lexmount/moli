#ifndef HEADER_CURL_MOLI_REFCOUNT_H
#define HEADER_CURL_MOLI_REFCOUNT_H
/* SPDX-License-Identifier: curl */

#include "easy_lock.h"

/* A removed easy may be cleaned up while its former multi reuses a cached
 * connection. Both retain peers and credentials. Protect their counts even
 * though the application never uses the same easy from two threads.
 *
 * Each including translation unit owns one object family and its lock. Only
 * the count is protected; no allocation, destruction or curl callbacks run
 * under the lock. Use curl's existing platform lock and single-thread fallback.
 */
#ifdef GLOBAL_INIT_IS_THREADSAFE
static curl_simple_lock moli_refcount_lock = CURL_SIMPLE_LOCK_INIT;
#endif

static CURL_INLINE void moli_refcount_link(uint32_t *count)
{
#ifdef GLOBAL_INIT_IS_THREADSAFE
  curl_simple_lock_lock(&moli_refcount_lock);
#endif
  DEBUGASSERT(*count && *count < UINT32_MAX);
  ++*count;
#ifdef GLOBAL_INIT_IS_THREADSAFE
  curl_simple_lock_unlock(&moli_refcount_lock);
#endif
}

static CURL_INLINE bool moli_refcount_unlink(uint32_t *count)
{
  bool last;
#ifdef GLOBAL_INIT_IS_THREADSAFE
  curl_simple_lock_lock(&moli_refcount_lock);
#endif
  DEBUGASSERT(*count);
  last = !--*count;
#ifdef GLOBAL_INIT_IS_THREADSAFE
  curl_simple_lock_unlock(&moli_refcount_lock);
#endif
  /* Do not read the count again: another owner may already have freed it. */
  return last;
}

#endif /* HEADER_CURL_MOLI_REFCOUNT_H */
