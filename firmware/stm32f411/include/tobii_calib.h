/*
 * Tobii calibration over TTP, for the STM32F411 gaze reader.
 *
 * A button press runs a calibration: the device collects gaze samples while the
 * user fixates known stimulus points, then computes and applies a calibration.
 * Command ids and the sequence are ported from crates/tobii-proto/src/
 * calibration.rs and crates/tobiid/src/calibration.rs (the source of truth).
 *
 * TTP calibration command ids (decimal; the TTP header "id" field):
 *   1010 START   1060 CLEAR   1110 WRITE   1030 COLLECT_2D (x,y + eye mask)
 *   1070 COMPUTE 1100 READ    1020 STOP
 */
#ifndef OPEN_TOBII_CALIB_H
#define OPEN_TOBII_CALIB_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define TOBII_CAL_START 1010u
#define TOBII_CAL_STOP 1020u
#define TOBII_CAL_COLLECT_2D 1030u
#define TOBII_CAL_CLEAR 1060u
#define TOBII_CAL_COMPUTE 1070u
#define TOBII_CAL_READ 1100u
#define TOBII_CAL_WRITE 1110u

#define TOBII_EYES_BOTH 3u

/* A calibration stimulus point, display-normalised in [0,1]. */
struct tobii_cal_point {
    float x;
    float y;
};

/*
 * Build one TTP command into `out` (channel 0, single bulk write):
 *   [u32 LE channel=0][u32 LE body_len][BE: 0x51, seq, 0, id, 0, payload_len][payload]
 * Returns the total length written, or 0 if it does not fit in `cap`.
 */
size_t tobii_build_command(uint8_t seq, uint32_t id, const uint8_t *payload, size_t plen,
                           uint8_t *out, size_t cap);

/*
 * Build a COLLECT_2D payload (`00 00` + fixed32 x + fixed32 y + u32 eye mask)
 * into `out`. Returns its length. x,y are display-normalised in [0,1].
 */
size_t tobii_collect_payload(float x, float y, uint32_t eye_mask, uint8_t *out, size_t cap);

/*
 * Run a calibration against the tracker over tobii_host_* :
 * START, CLEAR, COLLECT at each point (showing it and waiting `fixation_ms`),
 * COMPUTE, STOP. Returns true on success.
 *
 * NOTE: the exact prime/write/read-back steps and the compute acceptance are
 * simplified here; validate against crates/tobiid/src/calibration.rs and on
 * hardware. Multi-point calibration needs the user to fixate points across the
 * real field of view — see README (the OLED alone is too small to be the
 * stimulus surface).
 */
bool tobii_calib_run(const struct tobii_cal_point *points, size_t n, uint32_t fixation_ms);

#endif /* OPEN_TOBII_CALIB_H */
