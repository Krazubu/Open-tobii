/*
 * open-tobii STM32F411 gaze reader — application flow.
 *
 * Hardware-agnostic: talks to the tracker through tobii_host.h (STM32Cube
 * OTG_FS host) and decodes with tobii_proto.h. The OLED is optional and off by
 * default (config.h). See README.md for the design and milestones.
 */
#include "config.h"
#include "tobii_calib.h"
#include "tobii_host.h"
#include "tobii_init.h" /* generated: tools/gen_init_packets.py */
#include "tobii_proto.h"

#include <stdio.h>

/* Default calibration stimulus points (display-normalised), centre + corners.
 * The user must fixate each in turn while it is shown; see README on the
 * stimulus-surface limitation of the tiny OLED. */
static const struct tobii_cal_point cal_points[] = {
    {0.5f, 0.5f}, {0.1f, 0.1f}, {0.9f, 0.1f}, {0.9f, 0.9f}, {0.1f, 0.9f},
};
#define CAL_POINT_COUNT (sizeof cal_points / sizeof cal_points[0])

/* How long each point is shown before its sample is collected. */
#ifndef OPEN_TOBII_CAL_FIXATION_MS
#define OPEN_TOBII_CAL_FIXATION_MS 1500u
#endif

/* One bulk IN frame. The gaze (0x500) message is small; the image stream is
 * not read, so a few KB is plenty on 128 KB of SRAM. */
#define FRAME_BUF 2048

/* Replay the captured init on EP 0x05. */
static bool replay_init(void) {
    for (size_t i = 0; i < TOBII_INIT_PACKET_COUNT; i++) {
        if (tobii_host_bulk_out(tobii_init_packets[i].data, tobii_init_packets[i].len) < 0) {
            tobii_log("init replay: bulk OUT failed\n");
            return false;
        }
        /* The ET5/Spark answer each command on EP 0x83; a full driver drains
         * those between writes. TODO: read/ack as replay_init_packets does. */
    }
    return true;
}

int main(void) {
    /* TODO (milestone 2): HAL_Init(), clocks, OTG_FS host, I2C, UART — the
     * CubeMX-generated setup runs before this. */

    if (!tobii_host_open()) {
        tobii_log("no Tobii tracker\n");
        return 1;
    }

    /* Open the TTP channel (vendor request 65); this also starts EP 0x83. */
    if (tobii_host_control(false, TOBII_REQ_CHANNEL_OPEN, 0, 0, 0) < 0) {
        tobii_log("channel open (req 65) failed\n");
        return 1;
    }

    if (!replay_init()) return 1;

#if OPEN_TOBII_ENABLE_DISPLAY
    tobii_display_init();
#endif

    uint8_t frame[FRAME_BUF];
    for (;;) {
        /* A button press launches a calibration. */
        if (tobii_button_pressed()) {
            tobii_calib_run(cal_points, CAL_POINT_COUNT, OPEN_TOBII_CAL_FIXATION_MS);
        }

        int n = tobii_host_bulk_in(frame, sizeof frame, 1000);
        if (n <= 0) continue;

        struct tobii_gaze g;
        if (tobii_decode_gaze(frame, (size_t)n, &g)) {
#if OPEN_TOBII_ENABLE_GAZE_LOG
            char line[64];
            snprintf(line, sizeof line, "gaze %.3f %.3f %s\n", (double)g.x, (double)g.y,
                     g.valid ? "ok" : "--");
            tobii_log(line);
#endif
#if OPEN_TOBII_ENABLE_DISPLAY
            tobii_display_dot(g.x, g.y, g.valid);
#endif
        }
    }

    /* Unreached; on shutdown a full driver sends vendor request 66. */
}
