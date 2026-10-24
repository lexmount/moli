/* Local ABI boundary; x264.h and the library are built together. */
#include <stdint.h>
#include <stddef.h>
#include <x264.h>

typedef struct {
    const uint8_t *data;
    size_t len;
    int64_t pts;
    int64_t dts;
    int keyframe;
} moli_x264_output;

void *moli_x264_open(int width, int height, int threads) {
    x264_param_t param;
    if (x264_param_default_preset(&param, "veryfast", "zerolatency") < 0)
        return NULL;
    param.i_width = width;
    param.i_height = height;
    param.i_csp = X264_CSP_I420;
    param.i_bitdepth = 8;
    param.i_threads = threads;
    param.i_lookahead_threads = 1;
    param.b_opencl = 0;
    param.i_fps_num = 10;
    param.i_fps_den = 1;
    param.i_timebase_num = 1;
    param.i_timebase_den = 90000;
    param.b_vfr_input = 1;
    param.b_annexb = 0;
    param.b_repeat_headers = 0;
    param.i_bframe = 0;
    param.i_keyint_max = 20;
    param.i_log_level = X264_LOG_ERROR;
    param.rc.f_rf_constant = 23;
    param.vui.b_fullrange = 0;
    param.vui.i_colorprim = 1; /* BT.709 primaries */
    param.vui.i_colmatrix = 1; /* BT.709 matrix */
    param.vui.i_transfer = 13; /* IEC 61966-2-1 (sRGB) */
    if (x264_param_apply_profile(&param, "high") < 0)
        return NULL;
    return x264_encoder_open(&param);
}

int moli_x264_headers(void *encoder, const uint8_t **sps, int *sps_len,
                      const uint8_t **pps, int *pps_len) {
    x264_nal_t *nal;
    int count;
    *sps = NULL;
    *pps = NULL;
    if (x264_encoder_headers(encoder, &nal, &count) < 0)
        return -1;
    for (int i = 0; i < count; i++) {
        if (nal[i].i_payload <= 4)
            return -1;
        if (nal[i].i_type == NAL_SPS) {
            *sps = nal[i].p_payload + 4;
            *sps_len = nal[i].i_payload - 4;
        } else if (nal[i].i_type == NAL_PPS) {
            *pps = nal[i].p_payload + 4;
            *pps_len = nal[i].i_payload - 4;
        }
    }
    return *sps && *pps ? 0 : -1;
}

int moli_x264_encode(void *encoder, const uint8_t *y, const uint8_t *u,
                     const uint8_t *v, int width, int64_t pts, int idr,
                     moli_x264_output *output) {
    x264_picture_t input, picture;
    x264_nal_t *nal;
    int count;
    x264_picture_init(&input);
    input.img.i_csp = X264_CSP_I420;
    input.img.i_plane = 3;
    input.img.plane[0] = (uint8_t *)y;
    input.img.plane[1] = (uint8_t *)u;
    input.img.plane[2] = (uint8_t *)v;
    input.img.i_stride[0] = width;
    input.img.i_stride[1] = width / 2;
    input.img.i_stride[2] = width / 2;
    input.i_pts = pts;
    input.i_type = idr ? X264_TYPE_IDR : X264_TYPE_AUTO;
    int size = x264_encoder_encode(encoder, &nal, &count, y ? &input : NULL, &picture);
    if (size <= 0)
        return size;
    output->data = nal[0].p_payload;
    output->len = (size_t)size;
    output->pts = picture.i_pts;
    output->dts = picture.i_dts;
    output->keyframe = picture.b_keyframe;
    return size;
}

int moli_x264_delayed(void *encoder) {
    return x264_encoder_delayed_frames(encoder);
}

void moli_x264_close(void *encoder) {
    x264_encoder_close(encoder);
}
