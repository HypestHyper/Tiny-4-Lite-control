// SDK probe: calls one OBSBOT SDK function at a time, with timestamped markers, so the USB traffic each call
// causes can be read from a usbmon capture taken at the same time (run.sh does both). Our own code; it only
// uses the SDK from the local copy on this computer and ships nothing of OBSBOT's.
//
// Safety: reads, and changes that are put back straight away. Moving the gimbal only with --with-moves.
// Never: factory reset, presets added or deleted, start-up position, the camera's sleep timer, firmware.

#include "dev/devs.hpp"
#include "dev/dev.hpp"

#include <chrono>
#include <cstdio>
#include <cstring>
#include <functional>
#include <memory>
#include <string>
#include <thread>

static double now() {
    return std::chrono::duration<double>(std::chrono::system_clock::now().time_since_epoch()).count();
}
static void pause(int ms) { std::this_thread::sleep_for(std::chrono::milliseconds(ms)); }

static void hexdump(const char *what, const void *data, size_t size) {
    const unsigned char *bytes = static_cast<const unsigned char *>(data);
    std::printf("  %s (%zu bytes):", what, size);
    for (size_t i = 0; i < size; ++i) std::printf("%s%02x", i % 32 == 0 ? "\n    " : " ", bytes[i]);
    std::printf("\n");
}

// One step: markers before and after, the return code, and quiet time around it so its traffic stands apart.
static void step(const char *name, const std::function<int()> &call) {
    pause(1500);
    std::printf("STEP %s BEGIN %.6f\n", name, now());
    std::fflush(stdout);
    int ret = call();
    std::printf("STEP %s END %.6f ret=%d\n", name, now(), ret);
    std::fflush(stdout);
}

int main(int argc, char **argv) {
    bool moves = argc > 1 && std::strcmp(argv[1], "--with-moves") == 0;
    std::printf("PROBE START %.6f moves=%d\n", now(), moves ? 1 : 0);
    std::fflush(stdout);

    Devices::get().setEnableMdnsScan(false);
    std::shared_ptr<Device> dev;
    for (int i = 0; i < 100 && !dev; ++i) {
        for (auto &d : Devices::get().getDevList()) {
            auto type = d->productType();
            if (type == ObsbotProdTiny2Lite || type == ObsbotProdTiny2) { dev = d; break; }
        }
        if (!dev) pause(100);
    }
    if (!dev) { std::printf("NO CAMERA: no Tiny 2 / Tiny 2 Lite found by the SDK\n"); return 2; }
    // The serial number is left out: the report is meant to be shared.
    std::printf("CAMERA %s version=%s type=%d\n", dev->devName().c_str(), dev->devVersion().c_str(),
                static_cast<int>(dev->productType()));
    std::printf("CONNECTED %.6f\n", now());
    std::fflush(stdout);
    pause(3000);  // let the SDK's own start-up traffic finish

    // ── Reads ──
    step("get_camera_status", [&] { Device::CameraStatus s{}; int r = dev->cameraGetCameraStatusU(s); hexdump("CameraStatus", &s, sizeof s); return r; });
    step("get_ai_status", [&] { Device::AiStatus s{}; int r = dev->aiGetAiStatusR(&s); hexdump("AiStatus", &s, sizeof s); return r; });
    step("get_gimbal_state", [&] { Device::AiGimbalStateInfo s{}; int r = dev->aiGetGimbalStateR(&s); hexdump("AiGimbalStateInfo", &s, sizeof s); return r; });
    float att[3] = {0, 0, 0};
    step("get_gimbal_attitude", [&] { int r = dev->gimbalGetAttitudeInfoR(att); std::printf("  attitude %.2f %.2f %.2f\n", att[0], att[1], att[2]); return r; });
    step("get_zoom", [&] { float z = 0; int r = dev->cameraGetZoomAbsoluteR(z); std::printf("  zoom %.3f\n", z); return r; });
    step("get_boot_position", [&] { Device::PresetPosInfo p{}; int r = dev->aiGetGimbalBootPosR(&p); hexdump("PresetPosInfo", &p, sizeof p); return r; });
    step("get_preset_list", [&] { Device::DevDataArray a{}; int r = dev->aiGetGimbalPresetListR(&a); hexdump("DevDataArray", &a, sizeof a); return r; });
    step("get_zone_preset_list", [&] { Device::DevDataArray a{}; int r = dev->aiGetZonePresetListR(&a); hexdump("DevDataArray", &a, sizeof a); return r; });
    bool zone_on = false;
    step("get_limited_zone_enabled", [&] { int r = dev->aiGetLimitedZoneTrackEnabledR(zone_on); std::printf("  enabled %d\n", zone_on); return r; });
    step("get_limited_zone_auto_select", [&] { bool v = false; int r = dev->aiGetLimitedZoneTrackAutoSelectR(v); std::printf("  auto select %d\n", v); return r; });
    step("get_limited_zone_yaw_min", [&] { float d = 0; int r = dev->aiGetLimitedZoneTrackYawMinR(d, 0); std::printf("  yaw min %.2f\n", d); return r; });
    step("get_limited_zone_yaw_max", [&] { float d = 0; int r = dev->aiGetLimitedZoneTrackYawMaxR(d, 0); std::printf("  yaw max %.2f\n", d); return r; });
    step("get_limited_zone_pitch_min", [&] { float d = 0; int r = dev->aiGetLimitedZoneTrackPitchMinR(d, 0); std::printf("  pitch min %.2f\n", d); return r; });
    step("get_limited_zone_pitch_max", [&] { float d = 0; int r = dev->aiGetLimitedZoneTrackPitchMaxR(d, 0); std::printf("  pitch max %.2f\n", d); return r; });

    // ── Changes, each put back ──
    step("set_ai_auto_zoom_on", [&] { return dev->aiSetAiAutoZoomR(true); });
    step("set_ai_auto_zoom_off", [&] { return dev->aiSetAiAutoZoomR(false); });
    step("set_yaw_reverse_on", [&] { return dev->aiSetGimbalYawDirReverseR(true); });
    step("set_yaw_reverse_off", [&] { return dev->aiSetGimbalYawDirReverseR(false); });
    step("set_limited_zone_yaw_min_-30", [&] { return dev->aiSetLimitedZoneTrackYawMinR(-30.0f); });
    step("reset_limited_zone_yaw_min", [&] { return dev->aiRstLimitedZoneTrackYawMinR(); });
    step("set_limited_zone_yaw_max_30", [&] { return dev->aiSetLimitedZoneTrackYawMaxR(30.0f); });
    step("reset_limited_zone_yaw_max", [&] { return dev->aiRstLimitedZoneTrackYawMaxR(); });
    step("set_limited_zone_pitch_min_-10", [&] { return dev->aiSetLimitedZoneTrackPitchMinR(-10.0f); });
    step("reset_limited_zone_pitch_min", [&] { return dev->aiRstLimitedZoneTrackPitchMinR(); });
    step("set_limited_zone_pitch_max_10", [&] { return dev->aiSetLimitedZoneTrackPitchMaxR(10.0f); });
    step("reset_limited_zone_pitch_max", [&] { return dev->aiRstLimitedZoneTrackPitchMaxR(); });
    step("set_limited_zone_enabled_flip", [&] { return dev->aiSetLimitedZoneTrackEnabledR(!zone_on); });
    step("set_limited_zone_enabled_back", [&] { return dev->aiSetLimitedZoneTrackEnabledR(zone_on); });

    // ── Moves (only when asked): 10 degrees of pan and back, slowly ──
    if (moves) {
        step("move_pan_plus_10", [&] { return dev->gimbalSetSpeedPositionR(0.0f, att[1], att[2] + 10.0f, 0.0f, 20.0f, 20.0f); });
        pause(2500);
        step("move_pan_back", [&] { return dev->gimbalSetSpeedPositionR(0.0f, att[1], att[2], 0.0f, 20.0f, 20.0f); });
        pause(2500);
    }

    pause(1500);
    std::printf("PROBE END %.6f\n", now());
    std::fflush(stdout);
    Devices::get().close();
    return 0;
}
