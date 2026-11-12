/* Checks what Tobii's own Stream Engine sees through ttpbridge on Windows:
 * device discovery, device info, then a few seconds of gaze points.
 * The Stream Engine DLL is loaded at run time from the path given on the
 * command line (by default the one Tobii's software installs), so nothing
 * of Tobii's is needed to build it. Build it for the DLL's architecture:
 *   x86_64-w64-mingw32-clang -O2 -o se_probe.exe se_probe.c */
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <windows.h>

typedef struct {
    char serial_number[256], model[256], generation[256], firmware_version[256];
    char integration_id[128], hw_calibration_version[128], hw_calibration_date[128], lot_id[128];
    char integration_type[256], runtime_build_version[256];
} device_info_t;

typedef struct {
    int64_t timestamp_us;
    int validity;
    float position_xy[2];
} gaze_point_t;

typedef int (*api_create_fn)(void** api, const void* alloc, const void* log);
typedef int (*enumerate_fn)(void* api, void (*receiver)(const char*, void*), void* user);
typedef int (*device_create_fn)(void* api, const char* url, int field_of_use, void** device);
typedef int (*device_info_fn)(void* device, device_info_t* info);
typedef int (*gaze_subscribe_fn)(void* device, void (*cb)(const gaze_point_t*, void*), void* user);
typedef int (*wait_fn)(int count, void* const* devices);
typedef int (*process_fn)(void* device);

static char first_url[256];
static int gaze_count, gaze_valid;

static void on_url(const char* url, void* user) {
    (void)user;
    printf("  found %s\n", url);
    if (!first_url[0]) strncpy(first_url, url, sizeof first_url - 1);
}

static void on_gaze(const gaze_point_t* g, void* user) {
    (void)user;
    gaze_count++;
    if (g->validity) gaze_valid++;
    if (gaze_count <= 3 || gaze_count % 30 == 0)
        printf("  gaze #%d valid=%d (%.3f, %.3f) t=%lld\n", gaze_count, g->validity, g->position_xy[0],
               g->position_xy[1], (long long)g->timestamp_us);
}

int main(int argc, char** argv) {
    const char* dll = argc > 1 ? argv[1] : "C:\\Program Files\\Tobii\\Tobii EyeX\\tobii_stream_engine.dll";
    HMODULE h = LoadLibraryA(dll);
    if (!h) { printf("cannot load %s (error %lu)\n", dll, GetLastError()); return 1; }
#define SYM(t, n) t n = (t)(void*)GetProcAddress(h, #n); if (!n) { printf("missing %s\n", #n); return 1; }
    SYM(api_create_fn, tobii_api_create)
    SYM(enumerate_fn, tobii_enumerate_local_device_urls)
    SYM(device_create_fn, tobii_device_create)
    SYM(device_info_fn, tobii_get_device_info)
    SYM(gaze_subscribe_fn, tobii_gaze_point_subscribe)
    SYM(wait_fn, tobii_wait_for_callbacks)
    SYM(process_fn, tobii_device_process_callbacks)

    void* api = NULL;
    printf("tobii_api_create: %d\n", tobii_api_create(&api, NULL, NULL));
    printf("tobii_enumerate_local_device_urls: %d\n", tobii_enumerate_local_device_urls(api, on_url, NULL));
    if (!first_url[0]) return 1;

    void* dev = NULL;
    int st = tobii_device_create(api, first_url, 1 /* TOBII_FIELD_OF_USE_INTERACTIVE */, &dev);
    printf("tobii_device_create(%s): %d\n", first_url, st);
    if (st) return 1;

    device_info_t info;
    memset(&info, 0, sizeof info);
    st = tobii_get_device_info(dev, &info);
    printf("tobii_get_device_info: %d\n  serial %s\n  model %s\n  generation %s\n  firmware %s\n", st,
           info.serial_number, info.model, info.generation, info.firmware_version);

    printf("tobii_gaze_point_subscribe: %d\n", tobii_gaze_point_subscribe(dev, on_gaze, NULL));
    DWORD end = GetTickCount() + 5000;
    while (GetTickCount() < end) {
        tobii_wait_for_callbacks(1, &dev);
        tobii_device_process_callbacks(dev);
    }
    printf("gaze points in 5 s: %d (valid %d)\n", gaze_count, gaze_valid);
    return 0;
}
