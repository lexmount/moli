#ifndef MOLIO_IOS_H
#define MOLIO_IOS_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

int molio_v8_smoke(void);

typedef struct Molio Molio;
typedef void (*MolioPresent)(void *, const uint8_t *, size_t, uint32_t, uint32_t, size_t);
typedef void (*MolioStatus)(void *, const char *);
typedef void (*MolioEditor)(void *, uint32_t, const char *, bool, size_t, size_t, const float *);
typedef void (*MolioDownload)(void *, uint64_t, const char *, const char *, uint32_t, uint64_t, uint64_t);
typedef void (*MolioNavigation)(void *, const char *, const char *, bool, bool);

// Call on the UIKit main thread. Pass valid handles returned by this API.
// A null handle is permitted for molio_destroy. Callback data is borrowed
// for the duration of the callback. Do not call back into the same handle
// from a callback.
Molio *molio_create(const char *profile, const char *download_directory);
void molio_destroy(Molio *handle);
void molio_navigate(const Molio *handle, const char *url);
void molio_open_address(const Molio *handle, const char *input);
Molio *molio_new_tab(const Molio *handle);
Molio *molio_pop_tab(const Molio *handle);
bool molio_tab_closed(const Molio *handle);
void molio_download_command(const Molio *handle, uint64_t id, uint32_t action);
void molio_list_downloads(const Molio *handle, void *context, MolioDownload show);
void molio_insert_text(const Molio *handle, const char *utf8, size_t length, bool marked,
                       size_t selection_start, size_t selection_end);
void molio_finish_composition(const Molio *handle);
enum {
    MOLIO_EDITOR_BACKSPACE,
    MOLIO_EDITOR_ENTER,
    MOLIO_EDITOR_SELECT_ALL,
};
void molio_editor_key(const Molio *handle, uint8_t key);
void molio_set_active(const Molio *handle, bool active);
void molio_history(const Molio *handle, int32_t delta);
void molio_reload(const Molio *handle, bool stop);
void molio_resize(Molio *handle, uint32_t width, uint32_t height, double scale,
                  uint32_t screen_width, uint32_t screen_height);
void molio_tap(const Molio *handle, double x, double y);
void molio_scroll(const Molio *handle, double x, double y, double delta_x, double delta_y);
void molio_poll(Molio *handle, void *context, MolioPresent present, MolioStatus status,
                MolioNavigation navigation, MolioEditor editor);

#endif
