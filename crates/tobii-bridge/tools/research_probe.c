/* Checks what Eye Tracker Manager's SDK (libtobii_research) sees through
 * ttpbridge: discovery, identity, then a few seconds of gaze.
 * Copy libtobii_research.1.11.1.dylib from your PETM install
 * (Contents/Resources/stage/SdkJsBinding.app/Contents/Frameworks/) next to
 * this file as libtobii_research.dylib, then:
 *   cc -o research_probe research_probe.c -L. -ltobii_research -Wl,-rpath,@executable_path
 *   ./research_probe -v */
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

typedef struct TobiiResearchEyeTracker TobiiResearchEyeTracker;
typedef struct {
    TobiiResearchEyeTracker** eyetrackers;
    size_t count;
} TobiiResearchEyeTrackers;

int tobii_research_find_all_eyetrackers(TobiiResearchEyeTrackers** ets);
void tobii_research_free_eyetrackers(TobiiResearchEyeTrackers* ets);
int tobii_research_get_eyetracker(const char* address, TobiiResearchEyeTracker** et);
int tobii_research_get_address(TobiiResearchEyeTracker* et, char** s);
int tobii_research_get_serial_number(TobiiResearchEyeTracker* et, char** s);
int tobii_research_get_model(TobiiResearchEyeTracker* et, char** s);
int tobii_research_get_device_name(TobiiResearchEyeTracker* et, char** s);
int tobii_research_get_firmware_version(TobiiResearchEyeTracker* et, char** s);
void tobii_research_free_string(char* s);
int tobii_research_logging_subscribe(void (*cb)(int64_t, int, int, const char*));
int tobii_research_subscribe_to_gaze_data(TobiiResearchEyeTracker* et, void (*cb)(void*, void*), void* user);
int tobii_research_unsubscribe_from_gaze_data(TobiiResearchEyeTracker* et, void (*cb)(void*, void*));

static void on_log(int64_t ts, int source, int level, const char* msg) {
    (void)ts;
    printf("  [sdk log s%d l%d] %s\n", source, level, msg);
}

static int gaze_count;
static void on_gaze(void* data, void* user) {
    (void)data;
    (void)user;
    gaze_count++;
}

static void show(TobiiResearchEyeTracker* et) {
    char* s;
    if (tobii_research_get_address(et, &s) == 0) { printf("  address  %s\n", s); tobii_research_free_string(s); }
    if (tobii_research_get_serial_number(et, &s) == 0) { printf("  serial   %s\n", s); tobii_research_free_string(s); }
    if (tobii_research_get_model(et, &s) == 0) { printf("  model    %s\n", s); tobii_research_free_string(s); }
    if (tobii_research_get_device_name(et, &s) == 0) { printf("  name     %s\n", s); tobii_research_free_string(s); }
    if (tobii_research_get_firmware_version(et, &s) == 0) { printf("  firmware %s\n", s); tobii_research_free_string(s); }
}

int main(int argc, char** argv) {
    int verbose = argc > 1 && strcmp(argv[1], "-v") == 0;
    if (verbose) tobii_research_logging_subscribe(on_log);

    TobiiResearchEyeTracker* found = NULL;
    for (int round = 0; round < 4 && !found; round++) {
        TobiiResearchEyeTrackers* ets = NULL;
        int st = tobii_research_find_all_eyetrackers(&ets);
        printf("find_all_eyetrackers: status %d, %zu tracker(s)\n", st, ets ? ets->count : 0);
        if (ets) {
            for (size_t i = 0; i < ets->count; i++) {
                show(ets->eyetrackers[i]);
                if (!found) found = ets->eyetrackers[i];
            }
        }
        if (!found) sleep(2);
    }
    if (!found) return 1;

    int st = tobii_research_subscribe_to_gaze_data(found, on_gaze, NULL);
    printf("subscribe_to_gaze_data: status %d\n", st);
    if (st == 0) {
        sleep(4);
        tobii_research_unsubscribe_from_gaze_data(found, on_gaze);
        printf("gaze callbacks in 4 s: %d\n", gaze_count);
    }
    return 0;
}
