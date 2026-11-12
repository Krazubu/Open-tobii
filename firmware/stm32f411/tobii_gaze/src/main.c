/*
 * open-tobii STM32F411 USB-host gaze reader (TinyUSB).
 *
 * Enumerate the tracker (Full-Speed host), open its bulk pipes, run the vendor
 * control setup, replay the captured init, then read the gaze stream (0x500)
 * and draw the gaze point on the OLED.
 *
 * EP 0x83 is read with an always-pending async transfer (re-submitted from its
 * own callback) so the device's responses are drained while the init is
 * replayed on EP 0x05 — mirroring the daemon.
 *
 * OLED stages: 1 bar enumerated, 2 pipes+control, 3 init replayed, 4 first
 * EP 0x83 frame; a moving dot = a decoded gaze frame.
 *
 * The OLED is optional (OPEN_TOBII_ENABLE_DISPLAY, off by default; build with
 * `make OLED=1`). Without it the board LED is the only feedback: on once
 * the tracker is enumerated, blinking (~2 Hz) while valid gaze arrives.
 */
#include "bsp/board_api.h"
#include "tusb.h"

#include <string.h>

#include "config.h"
#if OPEN_TOBII_ENABLE_DISPLAY
#include "oled_hw.h"
#include "ssd1306.h"
#endif
#include "tobii_init.h"
#include "tobii_proto.h"

#if OPEN_TOBII_ENABLE_DISPLAY
static struct ssd1306 oled;
#endif
static uint8_t g_daddr = 0;
static volatile bool g_mounted = false;

// UI state updated from the EP 0x83 callback, rendered in the main loop.
static volatile uint8_t ui_stage = 0;
static volatile bool ui_have_gaze = false;
static volatile float ui_gx = 0, ui_gy = 0;

// ---- OLED ----------------------------------------------------------------
#if OPEN_TOBII_ENABLE_DISPLAY
static void draw_border(void) {
    for (int x = 0; x < SSD1306_W; x++) {
        ssd1306_set_pixel(&oled, x, 0, true);
        ssd1306_set_pixel(&oled, x, SSD1306_H - 1, true);
    }
    for (int y = 0; y < SSD1306_H; y++) {
        ssd1306_set_pixel(&oled, 0, y, true);
        ssd1306_set_pixel(&oled, SSD1306_W - 1, y, true);
    }
}
static void show_stage(int n) {
    ssd1306_clear(&oled);
    draw_border();
    for (int i = 0; i < n; i++) {
        int y0 = 6 + i * 8;
        for (int y = y0; y < y0 + 5; y++)
            for (int x = 8; x < SSD1306_W - 8; x++) ssd1306_set_pixel(&oled, x, y, true);
    }
    ssd1306_flush(&oled);
    board_led_write(n >= 1);
}
static void show_gaze(float x, float y) { ssd1306_show_gaze(&oled, x, y, true); }
#else
static void show_stage(int n) { board_led_write(n >= 1); }
static void show_gaze(float x, float y) {
    (void)x;
    (void)y;
    static uint32_t frames;
    board_led_write(((++frames >> 4) & 1u) == 0); // toggles every 16 frames
}
#endif

// ---- blocking helpers for control + OUT ----------------------------------
static volatile bool s_done;
static volatile xfer_result_t s_res;
static void on_blocking(tuh_xfer_t *x) {
    s_res = x->result;
    s_done = true;
}
static bool vendor_ctrl(uint8_t dir_in, uint8_t req, uint16_t wValue, uint8_t *buf, uint16_t len) {
    tusb_control_request_t setup = {.bmRequestType = dir_in ? 0xC1u : 0x41u, .bRequest = req,
                                    .wValue = wValue, .wIndex = 0, .wLength = len};
    tuh_xfer_t x = {.daddr = g_daddr, .ep_addr = 0, .setup = &setup, .buffer = buf, .complete_cb = on_blocking};
    s_done = false;
    if (!tuh_control_xfer(&x)) return false;
    while (!s_done) tuh_task();
    return s_res == XFER_RESULT_SUCCESS;
}
static bool bulk_out(uint8_t *buf, uint32_t len) {
    tuh_xfer_t x = {.daddr = g_daddr, .ep_addr = TOBII_EP_OUT, .buflen = len, .buffer = buf, .complete_cb = on_blocking};
    s_done = false;
    if (!tuh_edpt_xfer(&x)) return false;
    while (!s_done) tuh_task();
    return s_res == XFER_RESULT_SUCCESS;
}
static bool open_bulk(uint8_t addr, uint16_t mps) {
    tusb_desc_endpoint_t ep = {.bLength = sizeof(tusb_desc_endpoint_t), .bDescriptorType = TUSB_DESC_ENDPOINT,
                               .bEndpointAddress = addr, .bmAttributes = {.xfer = TUSB_XFER_BULK},
                               .wMaxPacketSize = mps, .bInterval = 0};
    return tuh_edpt_open(g_daddr, &ep);
}

// ---- always-pending EP 0x83 read ----------------------------------------
static uint8_t rx[2048];
static void on_rx(tuh_xfer_t *x);
static void submit_rx(void) {
    static tuh_xfer_t x;
    x = (tuh_xfer_t){.daddr = g_daddr, .ep_addr = TOBII_EP_IN, .buflen = sizeof rx, .buffer = rx, .complete_cb = on_rx};
    tuh_edpt_xfer(&x);
}
static void on_rx(tuh_xfer_t *x) {
    if (x->result == XFER_RESULT_SUCCESS && x->actual_len) {
        if (ui_stage < 4) ui_stage = 4;
        struct tobii_gaze g;
        if (tobii_decode_gaze(rx, x->actual_len, &g) && g.valid) {
            ui_gx = g.x;
            ui_gy = g.y;
            ui_have_gaze = true;
        }
    }
    submit_rx(); // keep a read pending
}

static uint8_t outbuf[4096];

static void run_tobii(void) {
    ui_stage = 1;
    show_stage(1);

    if (!open_bulk(TOBII_EP_IN, 64) || !open_bulk(TOBII_EP_OUT, 64)) return;

    uint8_t scp_req[24] = {1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0};
    uint8_t scp[512], r8[8];
    vendor_ctrl(0, 48, 0, scp_req, 24);
    vendor_ctrl(1, 48, 0, scp, sizeof scp);
    vendor_ctrl(1, 70, 1, r8, sizeof r8);
    if (!vendor_ctrl(0, 65, 0, NULL, 0)) return;
    ui_stage = 2;

    submit_rx(); // start draining EP 0x83 now, before/while we replay

    for (size_t i = 0; i < TOBII_INIT_PACKET_COUNT; i++) {
        size_t L = tobii_init_packets[i].len;
        if (L > sizeof outbuf) return;
        memcpy(outbuf, tobii_init_packets[i].data, L);
        if (!bulk_out(outbuf, (uint32_t)L)) return;
    }
    if (ui_stage < 3) ui_stage = 3;

    // Render loop: show the dot when gaze arrives, else the furthest stage.
    uint8_t shown_stage = 0;
    bool showing_dot = false;
    for (;;) {
        tuh_task();
        if (ui_have_gaze) {
            ui_have_gaze = false;
            show_gaze(ui_gx, ui_gy);
            showing_dot = true;
            shown_stage = 0;
        } else if (!showing_dot && ui_stage != shown_stage) {
            shown_stage = ui_stage;
            show_stage(shown_stage);
        }
    }
}

int main(void) {
    board_init();
#if OPEN_TOBII_ENABLE_DISPLAY
    oled_i2c_init();
    ssd1306_init(&oled, oled_i2c_write, OPEN_TOBII_OLED_I2C_ADDR);
#endif
    show_stage(0);

    tusb_rhport_init_t host_init = {.role = TUSB_ROLE_HOST, .speed = TUSB_SPEED_AUTO};
    tusb_init(BOARD_TUH_RHPORT, &host_init);
    board_init_after_tusb();

    for (;;) {
        tuh_task();
        if (g_mounted) {
            g_mounted = false;
            run_tobii();
        }
    }
}

void tuh_mount_cb(uint8_t daddr) {
    uint16_t vid = 0, pid = 0;
    if (tuh_vid_pid_get(daddr, &vid, &pid) && vid == TOBII_USB_VID) {
        g_daddr = daddr;
        g_mounted = true;
    }
}
void tuh_umount_cb(uint8_t daddr) { (void)daddr; }
