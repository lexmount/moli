#ifndef MOLIO_BROWSER_H
#define MOLIO_BROWSER_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct MolioBrowser MolioBrowser;

/* Open with an absolute, writable app-private profile path. */
MolioBrowser* molio_browser_open(const char* profile);
/* Stop and join the presentation thread before closing the browser. */
void molio_browser_close(MolioBrowser* browser);
bool molio_browser_send(const MolioBrowser* browser, const char* json);

/* State JSON and packed BGRA pixels are borrowed until the callback returns.
 * Callbacks must not reenter the browser. */
void molio_browser_state(const MolioBrowser* browser, uint64_t after,
    void (*callback)(void*, const uint8_t*, size_t), void* context);
uint64_t molio_browser_wait_frame(const MolioBrowser* browser, uint64_t after);
bool molio_browser_frame(const MolioBrowser* browser,
    bool (*callback)(void*, const uint8_t*, uint32_t, uint32_t), void* context);

#ifdef __cplusplus
}
#endif

#endif
