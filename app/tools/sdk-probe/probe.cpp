// SDK probe: calls OBSBOT SDK functions one at a time, with timestamped markers, so the USB traffic each call
// causes can be read from a usbmon capture taken at the same time (run.sh does both). Our own code; it only
// uses the SDK from the local copy on this computer and ships nothing of OBSBOT's.
//
// Three levels, chosen with flags:
//   (default)   every read; settings whose value can be read first are changed and written back exactly;
//               picture, focus, exposure, white balance, anti-flicker and zoom are written with the value they
//               already have; HDR, face exposure and the target gesture are flipped and restored from the
//               camera's status bytes.
//   --toggles   on/off settings whose state cannot be read: each ends in a stated state, listed at the end.
//   --moves     anything that moves the camera or changes what it is doing (pans, zoom, presets, AI modes,
//               sleep and wake). Each move is undone; AI tracking ends on "no tracking".
//   --hidden    instead of all the above: the functions in the SDK library that its headers leave out (found by
//               tools/sdk-probe/inventory.py) and that take plain numbers, called by their exact signature. Reads,
//               then each setting written back with the value read (numbers and on/off are changed for a moment
//               and restored). With --toggles also: hand-tracking area set then reset to default, the hidden
//               gesture switches (on, then off), the focus point at the centre. With --moves also: zoom by the
//               hidden zoom command, and select the target at the picture's centre. Functions whose parameters
//               are undocumented structures, or whose argument order is unknown (Euler angles), are not called.
//   --round5    instead of all the above: zone tracking switched on and off (aiSetZoneTrackStateR, the limited
//               zone switch, the gimbal's auto-tracking and the LED pattern) with the AI status read after each and
//               the state restored; the start-up position and each saved position written back with the values
//               read (aiSetGimbalBootPosR, aiUpdGimbalPresetR, aiSetGimbalPresetNameWithIdR). With --toggles also:
//               the last saved position deleted and added back as it was (aiDelGimbalPresetR, aiAddGimbalPresetR).
//   --mic       instead of all the above: the microphone switched off and on again with OBSBOT's own function
//               (cameraSetAudioMicDisabledU), so its bytes can be compared with this app's; it ends switched on.
//   --round4    instead of all the above: the zone return position written with a new value, read back and
//               restored; the hand side (right, left) and each gesture switch changed one at a time with the AI
//               status read after each, then restored from the first reading. With --moves also: the camera
//               sent to the return position. With --sleep-images also (and only then): the thumbnails of the
//               camera's three sleep pictures downloaded to ~/obsbot-sleep-images (read only; it may take the
//               camera a moment).
// Never: factory reset; adding, deleting, renaming or updating presets or zone presets; setting or resetting the
// start-up position or the zone start position; background images; the boot AI mode; privacy mode; firmware.

#include "dev/devs.hpp"
#include "dev/dev.hpp"

#include <cxxabi.h>
#include <dlfcn.h>
#include <link.h>
#include <atomic>
#include <cstdlib>
#include <map>

#include <chrono>
#include <cstdio>
#include <cstring>
#include <functional>
#include <memory>
#include <string>
#include <thread>
#include <vector>

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
static int step(const std::string &name, const std::function<int()> &call, int gap_ms = 1200) {
    pause(gap_ms);
    std::printf("STEP %s BEGIN %.6f\n", name.c_str(), now());
    std::fflush(stdout);
    int ret = call();
    std::printf("STEP %s END %.6f ret=%d\n", name.c_str(), now(), ret);
    std::fflush(stdout);
    return ret;
}

static std::vector<std::string> final_states;
static void finally(const std::string &what) { final_states.push_back(what); }

static void range(const std::string &name, Device::UvcParamRange &r) {
    std::printf("  range min %ld max %ld step %ld default %ld\n", r.min_, r.max_, r.step_, r.default_);
}

// ════════════ Functions the headers leave out ════════════
// The library exports them; we find each by its demangled signature (as inventory.py prints it) and call it
// with plain types. Enumerations are passed as int, which is how the compiler passes them.
static std::map<std::string, void *> hidden_symbols;
static void load_hidden_symbols() {
    std::string lib;
    dl_iterate_phdr([](dl_phdr_info *info, size_t, void *out) {
        if (info->dlpi_name && std::strstr(info->dlpi_name, "libdev.so")) { *static_cast<std::string *>(out) = info->dlpi_name; return 1; }
        return 0;
    }, &lib);
    if (lib.empty()) { std::printf("HIDDEN: cannot locate libdev\n"); return; }
    std::string cmd = "nm -D --defined-only '" + lib + "'";
    FILE *nm = popen(cmd.c_str(), "r");
    if (!nm) { std::printf("HIDDEN: nm not available (install binutils)\n"); return; }
    char line[4096];
    while (std::fgets(line, sizeof line, nm)) {
        char *name = std::strrchr(line, ' ');
        if (!name) continue;
        std::string mangled(name + 1);
        while (!mangled.empty() && (mangled.back() == '\n' || mangled.back() == '\r')) mangled.pop_back();
        int ok = 0;
        char *plain = abi::__cxa_demangle(mangled.c_str(), nullptr, nullptr, &ok);
        if (ok == 0 && plain) {
            if (void *at = dlsym(RTLD_DEFAULT, mangled.c_str())) hidden_symbols[plain] = at;
        }
        std::free(plain);
    }
    pclose(nm);
    std::printf("HIDDEN: %zu symbols from %s\n", hidden_symbols.size(), lib.c_str());
}

template <class... A> static int hidden(Device *dev, const char *signature, A... args) {
    auto it = hidden_symbols.find(signature);
    if (it == hidden_symbols.end()) { std::printf("  not in this library: %s\n", signature); return -999; }
    return reinterpret_cast<int32_t (*)(Device *, A...)>(it->second)(dev, args...);
}

static void run_hidden(Device *d, bool toggles, bool moves) {
    load_hidden_symbols();

    // Device sleep after inactivity (the R-command; cameraSetSuspendTimeU is the status-block one).
    unsigned int sleep_time = 0;
    if (step("hidden_get_auto_sleep_time", [&] { int r = hidden(d, "Device::cameraGetAutoSleepTimeR(unsigned int&)", &sleep_time); std::printf("  auto sleep %u\n", sleep_time); return r; }) == 0) {
        unsigned int other = sleep_time == 600 ? 300 : 600;
        step("hidden_set_auto_sleep_time_" + std::to_string(other), [&] { return hidden(d, "Device::cameraSetAutoSleepTimeR(unsigned int)", other); });
        step("hidden_set_auto_sleep_time_back", [&] { return hidden(d, "Device::cameraSetAutoSleepTimeR(unsigned int)", sleep_time); });
    }

    // Audio.
    short volume = 0;
    if (step("hidden_get_audio_volume", [&] { int r = hidden(d, "Device::cameraGetAudioVolumeR(short&)", &volume); std::printf("  volume %d\n", volume); return r; }) == 0) {
        short other = volume > 0 ? volume - 1 : volume + 1;
        step("hidden_set_audio_volume_" + std::to_string(other), [&] { return hidden(d, "Device::cameraSetAudioVolumeR(short)", other); });
        step("hidden_set_audio_volume_back", [&] { return hidden(d, "Device::cameraSetAudioVolumeR(short)", volume); });
    }
    bool agc = false;
    if (step("hidden_get_audio_agc", [&] { int r = hidden(d, "Device::cameraGetAudioAGCR(bool&)", &agc); std::printf("  agc %d\n", agc); return r; }) == 0) {
        step("hidden_set_audio_agc_flip", [&] { return hidden(d, "Device::cameraSetAudioAGCR(bool)", !agc); });
        step("hidden_set_audio_agc_back", [&] { return hidden(d, "Device::cameraSetAudioAGCR(bool)", agc); });
    }
    bool nr_on = false; int nr_level = 0;
    if (step("hidden_get_audio_noise_reduce", [&] { int r = hidden(d, "Device::cameraGetAudioNoiseReduceR(bool&, int&)", &nr_on, &nr_level); std::printf("  noise reduction %d level %d\n", nr_on, nr_level); return r; }) == 0) {
        step("hidden_set_audio_noise_reduce_flip", [&] { return hidden(d, "Device::cameraSetAudioNoiseReduceR(bool, int)", !nr_on, nr_level); });
        step("hidden_set_audio_noise_reduce_back", [&] { return hidden(d, "Device::cameraSetAudioNoiseReduceR(bool, int)", nr_on, nr_level); });
    }
    bool muted = false;
    if (step("hidden_get_audio_mute", [&] { int r = hidden(d, "Device::cameraGetAudioSourceMuteR(bool&)", &muted); std::printf("  mute %d\n", muted); return r; }) == 0) {
        step("hidden_set_audio_mute_flip", [&] { return hidden(d, "Device::cameraSetAudioSourceMuteR(bool)", !muted); });
        step("hidden_set_audio_mute_back", [&] { return hidden(d, "Device::cameraSetAudioSourceMuteR(bool)", muted); });
    }

    // Picture modes whose values are enumerations we do not know: read, and written back unchanged.
    struct Enum { const char *name, *get, *set; };
    Enum enums[] = {
        {"gamma_mode", "Device::cameraGetGammaModeR(Device::DevGammaMode&)", "Device::cameraSetGammaModeR(Device::DevGammaMode)"},
        {"night_mode", "Device::cameraGetNightModeR(Device::DevNightMode&)", "Device::cameraSetNightModeR(Device::DevNightMode)"},
        {"image_style", "Device::cameraGetImageStyleModeR(Device::DevImageStyleMode&)", "Device::cameraSetImageStyleModeR(Device::DevImageStyleMode)"},
    };
    for (auto &e : enums) {
        int value = 0;
        if (step(std::string("hidden_get_") + e.name, [&] { int r = hidden(d, e.get, &value); std::printf("  %s %d\n", e.name, value); return r; }) == 0)
            step(std::string("hidden_set_") + e.name + "_same", [&] { return hidden(d, e.set, value); });
    }
    bool comp = false;
    if (step("hidden_get_auto_compensation", [&] { int r = hidden(d, "Device::cameraGetAutoCompensationR(bool&)", &comp); std::printf("  auto compensation %d\n", comp); return r; }) == 0) {
        step("hidden_set_auto_compensation_flip", [&] { return hidden(d, "Device::cameraSetAutoCompensationR(bool)", !comp); });
        step("hidden_set_auto_compensation_back", [&] { return hidden(d, "Device::cameraSetAutoCompensationR(bool)", comp); });
    }
    int wb_a = 0, wb_b = 0; bool wb_c = false;
    if (step("hidden_get_white_balance_component", [&] { int r = hidden(d, "Device::cameraGetWhiteBalanceComponentR(int&, int&, bool&)", &wb_a, &wb_b, &wb_c); std::printf("  component %d %d %d\n", wb_a, wb_b, wb_c); return r; }) == 0)
        step("hidden_set_white_balance_component_same", [&] { return hidden(d, "Device::cameraSetWhiteBalanceComponentR(int, int, bool)", wb_a, wb_b, wb_c); });
    float area[4] = {0, 0, 0, 0};
    if (step("hidden_get_white_balance_area", [&] { int r = hidden(d, "Device::cameraGetWhiteBalanceAreaR(float&, float&, float&, float&)", &area[0], &area[1], &area[2], &area[3]); std::printf("  area %.3f %.3f %.3f %.3f\n", area[0], area[1], area[2], area[3]); return r; }) == 0)
        step("hidden_set_white_balance_area_same", [&] { return hidden(d, "Device::cameraSetWhiteBalanceAreaR(float, float, float, float)", area[0], area[1], area[2], area[3]); });
    int view[4] = {0, 0, 0, 0};
    if (step("hidden_get_field_view", [&] { int r = hidden(d, "Device::cameraGetFieldView(int&, int&, int&, int&)", &view[0], &view[1], &view[2], &view[3]); std::printf("  field view %d %d %d %d\n", view[0], view[1], view[2], view[3]); return r; }) == 0)
        step("hidden_set_field_view_same", [&] { return hidden(d, "Device::cameraSetFieldView(unsigned int, unsigned int, unsigned int, unsigned int)", unsigned(view[0]), unsigned(view[1]), unsigned(view[2]), unsigned(view[3])); });
    step("hidden_get_uvc_mode", [&] { unsigned char m = 0; int r = hidden(d, "Device::cameraGetUvcModeR(unsigned char&)", &m); std::printf("  uvc mode %d\n", m); return r; });
    bool vt = false;
    if (step("hidden_get_virtual_track", [&] { int r = hidden(d, "Device::cameraGetVirtualTrackEnabledR(bool&)", &vt); std::printf("  virtual track %d\n", vt); return r; }) == 0)
        step("hidden_set_virtual_track_same", [&] { return hidden(d, "Device::cameraSetVirtualTrackEnabledR(bool)", vt); });

    // Status light.
    bool led = false;
    if (step("hidden_get_led_enabled", [&] { int r = hidden(d, "Device::sysMgGetLedEnabledR(bool&)", &led); std::printf("  led %d\n", led); return r; }) == 0) {
        step("hidden_set_led_flip", [&] { return hidden(d, "Device::sysMgSetLedEnabledR(bool)", !led); });
        step("hidden_set_led_back", [&] { return hidden(d, "Device::sysMgSetLedEnabledR(bool)", led); });
    }
    unsigned char bright = 0;
    if (step("hidden_get_led_brightness", [&] { int r = hidden(d, "Device::sysMgGetLedBrightnessR(unsigned char&)", &bright); std::printf("  led brightness %d\n", bright); return r; }) == 0) {
        unsigned char other = bright > 10 ? bright - 10 : bright + 10;
        step("hidden_set_led_brightness_" + std::to_string(other), [&] { return hidden(d, "Device::sysMgSetLedBrightnessR(unsigned char)", other); });
        step("hidden_set_led_brightness_back", [&] { return hidden(d, "Device::sysMgSetLedBrightnessR(unsigned char)", bright); });
    }

    // Zone tracking's return position ("After exceeding the tracking range, the gimbal returns to the set
    // position" in OBSBOT Center): read, the default, and written back with the value read.
    Device::PresetPosInfo home{};
    step("get_limited_zone_init_pos_default", [&] { Device::PresetPosInfo p{}; int r = d->aiGetLimitedZoneTrackInitPosR(&p, -1); std::printf("  default init id %d pitch %.2f yaw %.2f roll %.2f zoom %.2f\n", p.id, p.pitch, p.yaw, p.roll, p.zoom); return r; });
    if (step("get_limited_zone_init_pos", [&] { int r = d->aiGetLimitedZoneTrackInitPosR(&home, 0); std::printf("  init id %d pitch %.2f yaw %.2f roll %.2f zoom %.2f\n", home.id, home.pitch, home.yaw, home.roll, home.zoom); hexdump("PresetPosInfo", &home, sizeof home); return r; }) == 0)
        step("set_limited_zone_init_pos_same", [&] { return d->aiSetLimitedZoneTrackInitPosR(&home); });

    if (toggles) {
        struct Zone { const char *name, *set, *reset; float value; };
        Zone zones[] = {
            {"yaw_min", "Device::aiSetHandZoneYawMinR(float)", "Device::aiRstHandZoneYawMinR()", -30.0f},
            {"yaw_max", "Device::aiSetHandZoneYawMaxR(float)", "Device::aiRstHandZoneYawMaxR()", 30.0f},
            {"pitch_min", "Device::aiSetHandZonePitchMinR(float)", "Device::aiRstHandZonePitchMinR()", -10.0f},
            {"pitch_max", "Device::aiSetHandZonePitchMaxR(float)", "Device::aiRstHandZonePitchMaxR()", 10.0f},
        };
        for (auto &z : zones) {
            step(std::string("hidden_set_hand_zone_") + z.name, [&] { return hidden(d, z.set, z.value); });
            step(std::string("hidden_reset_hand_zone_") + z.name, [&] { return hidden(d, z.reset); });
        }
        finally("Hand-tracking area: OBSBOT's default");
        const char *switches[][2] = {
            {"gesture_zoom", "Device::aiSetGestureZoomR(bool)"},
            {"gesture_target_select", "Device::aiSetGestureTargetSelectR(bool)"},
            {"gesture_dynamic_zoom", "Device::aiSetGestureDynamicZoomR(bool)"},
            {"dynamic_zoom_mirror", "Device::aiSetDynamicZoomMirrorR(bool)"},
            {"hand_track_gimbal", "Device::aiSetHandTrackGimbalEnabledR(bool)"},
        };
        for (auto &s : switches) {
            step(std::string("hidden_set_") + s[0] + "_on", [&] { return hidden(d, s[1], true); });
            step(std::string("hidden_set_") + s[0] + "_off", [&] { return hidden(d, s[1], false); });
        }
        finally("Gestures (zoom, target selection, dynamic zoom, reverse dynamic zoom): off");
        step("zone_track_gimbal_off", [&] { return d->aiSetZoneTrackGimbalEnabledR(false); });
        step("zone_track_gimbal_on", [&] { return d->aiSetZoneTrackGimbalEnabledR(true); });
        step("hidden_focus_point_centre", [&] { return hidden(d, "Device::cameraSetUserFocusPos(float, float)", 0.5f, 0.5f); });
        finally("Focus point: centre of the picture");
    }

    if (moves) {
        float zoom = 1.0f;
        d->cameraGetZoomAbsoluteR(zoom);
        step("hidden_zoom_ratio_2", [&] { return hidden(d, "Device::aiSetCameraZoomRatioR(float, int)", 2.0f, -1); });
        pause(2500);
        step("hidden_zoom_ratio_read", [&] { float z = 0; int r = d->cameraGetZoomAbsoluteR(z); std::printf("  zoom now %.3f\n", z); return r; });
        step("hidden_zoom_back", [&] { return d->cameraSetZoomAbsoluteR(zoom); });
        pause(2500);
        step("hidden_select_target_centre", [&] { return hidden(d, "Device::aiSetSelectTargetByPos(float, float, int)", 0.5f, 0.5f, 0); }, 2000);
        step("hidden_ai_mode_none", [&] { return d->cameraSetAiModeU(Device::AiWorkModeNone, 0); }, 2000);
        finally("AI tracking: no tracking; zoom as before");
    }
}

// ════════════ Round 4: return position, hand side, gesture read-back, sleep pictures ════════════
static void ai_status(Device *d, const std::string &name) {
    step(name, [&] { Device::AiStatus s{}; int r = d->aiGetAiStatusR(&s); hexdump("AiStatus", &s, sizeof s);
        std::printf("  target %d zoom %d dynamic %d mirror %d factor %.2f yaw_reverse %d hand %d zone %d vtrack %d\n",
                    s.gesture_target, s.gesture_zoom, s.gesture_dynamic_zoom, s.gesture_mirror, s.gesture_zoom_factor,
                    s.yaw_reverse, static_cast<int>(s.hand_track_type), s.ai_zone_track, static_cast<int>(s.v_track_landscape));
        return r; }, 800);
}

static void run_round4(Device *d, bool moves, bool images) {
    load_hidden_symbols();

    // Return position: a different value, read back, then the one read first.
    Device::PresetPosInfo home{};
    if (step("r4_get_return_position", [&] { int r = d->aiGetLimitedZoneTrackInitPosR(&home, 0); std::printf("  return id %d roll %.2f pitch %.2f yaw %.2f zoom %.2f\n", home.id, home.roll, home.pitch, home.yaw, home.zoom); return r; }) == 0) {
        Device::PresetPosInfo other = home;
        other.yaw = 20.0f; other.pitch = -10.0f; other.zoom = 1.5f;
        step("r4_set_return_position_yaw20_pitch-10_zoom1.5", [&] { return d->aiSetLimitedZoneTrackInitPosR(&other); });
        step("r4_get_return_position_after", [&] { Device::PresetPosInfo p{}; int r = d->aiGetLimitedZoneTrackInitPosR(&p, 0); std::printf("  return now pitch %.2f yaw %.2f zoom %.2f\n", p.pitch, p.yaw, p.zoom); return r; });
        if (moves) {
            float att[3] = {0, 0, 0};
            d->gimbalGetAttitudeInfoR(att);
            step("r4_move_to_return_position", [&] { return d->aiTrgLimitedZoneTrackInitPosR(); }, 1500);
            pause(3500);
            step("r4_attitude_at_return_position", [&] { float a[3] = {0, 0, 0}; int r = d->gimbalGetAttitudeInfoR(a); std::printf("  attitude pitch %.2f yaw %.2f\n", a[1], a[2]); return r; });
            step("r4_move_back", [&] { return d->aiSetGimbalMotorAngleR(att[1], att[2]); });
            pause(3000);
            step("r4_zoom_back", [&] { return d->cameraSetZoomAbsoluteR(1.0f); });
            finally("Zoom: 1x (the return position test zoomed in)");
        }
        step("r4_set_return_position_back", [&] { return d->aiSetLimitedZoneTrackInitPosR(&home); });
        step("r4_get_return_position_restored", [&] { Device::PresetPosInfo p{}; int r = d->aiGetLimitedZoneTrackInitPosR(&p, 0); std::printf("  return now pitch %.2f yaw %.2f zoom %.2f\n", p.pitch, p.yaw, p.zoom); return r; });
    }

    // The AI status before anything changes, to restore from.
    Device::AiStatus first{};
    d->aiGetAiStatusR(&first);
    ai_status(d, "r4_ai_status_start");

    // Hand side: AiHandTrackRight = 0, AiHandTrackLeft = 1 (SDK header).
    const char *hand = "Device::aiSetHandTrackModeR(Device::AiHandTrackType)";
    step("r4_hand_left", [&] { return hidden(d, hand, 1); });
    ai_status(d, "r4_ai_status_hand_left");
    step("r4_hand_right", [&] { return hidden(d, hand, 0); });
    ai_status(d, "r4_ai_status_hand_right");
    step("r4_hand_back", [&] { return hidden(d, hand, static_cast<int>(first.hand_track_type)); });
    step("r4_hand_state", [&] {
        Device::AiHandTrackStateInfo s{};
        int r = hidden(d, "Device::aiGetHandTrackStateR(Device::AiHandTrackStateInfo*, int, std::function<void (void*, void const*)>, void*, Device::GetMethod)",
                       &s, 0, Device::RxDataCallback(), static_cast<void *>(nullptr), static_cast<int>(Device::Block));
        std::printf("  hand area yaw %.2f..%.2f pitch %.2f..%.2f view %d hand %d\n", s.yaw_min, s.yaw_max, s.pitch_min, s.pitch_max, s.view_id, static_cast<int>(s.hand_type));
        return r; });

    // Each gesture switch flipped and the status read, so its byte in the reply can be found; then restored.
    const bool was[4] = {first.gesture_target, first.gesture_zoom, first.gesture_dynamic_zoom, first.gesture_mirror};
    const int ids[4] = {0, 1, 2, 3};
    const char *names[4] = {"target", "zoom", "dynamic_zoom", "mirror"};
    for (int i = 0; i < 4; ++i) {
        step(std::string("r4_gesture_") + names[i] + "_flip", [&] { return d->aiSetGestureCtrlIndividualR(ids[i], !was[i]); });
        ai_status(d, std::string("r4_ai_status_") + names[i] + "_flipped");
        step(std::string("r4_gesture_") + names[i] + "_back", [&] { return d->aiSetGestureCtrlIndividualR(ids[i], was[i]); });
    }
    const char *factor = "Device::aiSetGestureZoomFactorR(float)";
    float other_factor = first.gesture_zoom_factor > 1.9f && first.gesture_zoom_factor < 2.1f ? 3.0f : 2.0f;
    step("r4_gesture_zoom_factor_other", [&] { return hidden(d, factor, other_factor); });
    ai_status(d, "r4_ai_status_factor_changed");
    step("r4_gesture_zoom_factor_back", [&] { return hidden(d, factor, first.gesture_zoom_factor); });
    ai_status(d, "r4_ai_status_end");

    if (images) {
        std::string dir = std::string(std::getenv("HOME") ? std::getenv("HOME") : ".") + "/obsbot-sleep-images";
        std::string mkdir = "mkdir -p '" + dir + "'";
        if (std::system(mkdir.c_str()) != 0) std::printf("  could not create %s\n", dir.c_str());
        static std::atomic<int> done{0};
        d->setFileDownloadCallback([](void *, uint32_t type, int32_t result, void *) {
            std::printf("  download callback: type 0x%x result %d\n", type, result); std::fflush(stdout); done.fetch_add(1);
        }, nullptr);
        for (uint32_t i = 0; i < 3; ++i) {
            d->setLocalResourcePath(dir + "/mini" + std::to_string(i) + ".jpg", dir + "/image" + std::to_string(i) + ".jpg", i);
        }
        uint32_t minis = Device::DownloadImageMini0 | Device::DownloadImageMini1 | Device::DownloadImageMini2;
        step("r4_download_sleep_thumbnails", [&] { return d->startFileDownloadAsync(static_cast<Device::FileType>(minis)); });
        for (int i = 0; i < 300 && done.load() < 3; ++i) pause(100);
        step("r4_status_after_download", [&] { Device::CameraStatus s{}; int r = d->cameraGetCameraStatusU(s); hexdump("CameraStatus", &s, sizeof s); return r; });
        for (uint32_t i = 0; i < 3; ++i) std::printf("  thumbnail %u: %s\n", i, d->localFileMiniPath(i).c_str());
    }
}

// ════════════ Round 5: zone tracking switches, start-up position and saved positions written back ════════════
static void run_round5(Device *d, bool toggles) {
    Device::AiStatus first{};
    d->aiGetAiStatusR(&first);
    ai_status(d, "r5_ai_status_start");
    std::printf("  zone track at start: %d\n", first.ai_zone_track);
    step("r5_zone_track_state_on", [&] { return d->aiSetZoneTrackStateR(true); }, 1500);
    ai_status(d, "r5_ai_status_zone_on");
    step("r5_limited_zone_enabled_on", [&] { return d->aiSetLimitedZoneTrackEnabledR(true); });
    ai_status(d, "r5_ai_status_limited_on");
    step("r5_limited_zone_enabled_off", [&] { return d->aiSetLimitedZoneTrackEnabledR(false); });
    step("r5_zone_track_state_off", [&] { return d->aiSetZoneTrackStateR(false); }, 1500);
    ai_status(d, "r5_ai_status_zone_off");
    step("r5_zone_track_state_back", [&] { return d->aiSetZoneTrackStateR(first.ai_zone_track != 0); });
    // The order the SDK documents for setting a zone: LED pattern on, gimbal auto-tracking off, then both back.
    step("r5_led_pattern_on", [&] { return d->cameraSetLedCtrlU(true); });
    step("r5_zone_gimbal_off", [&] { return d->aiSetZoneTrackGimbalEnabledR(false); });
    step("r5_zone_gimbal_on", [&] { return d->aiSetZoneTrackGimbalEnabledR(true); });
    step("r5_led_pattern_off", [&] { return d->cameraSetLedCtrlU(false); });

    // Start-up position: read, and written back as it is.
    Device::PresetPosInfo boot{};
    if (step("r5_get_boot_position", [&] { int r = d->aiGetGimbalBootPosR(&boot); std::printf("  boot id %d pitch %.2f yaw %.2f zoom %.2f\n", boot.id, boot.pitch, boot.yaw, boot.zoom); hexdump("PresetPosInfo", &boot, sizeof boot); return r; }) == 0)
        step("r5_set_boot_position_same", [&] { return d->aiSetGimbalBootPosR(boot, false); });

    // Saved positions: each read, then written back as it is (position, then name).
    Device::DevDataArray ids{};
    step("r5_get_preset_list", [&] { int r = d->aiGetGimbalPresetListR(&ids); std::printf("  %d preset(s)\n", ids.len); return r; });
    std::vector<int32_t> preset_ids;
    for (int i = 0; i < ids.len && i < 16; ++i) preset_ids.push_back(ids.data_int32[i]);
    Device::PresetPosInfo last{};
    for (int32_t id : preset_ids) {
        Device::PresetPosInfo info{};
        Device::DevDataArray name{};
        if (step("r5_get_preset_" + std::to_string(id), [&] { int r = d->aiGetGimbalPresetInfoWithIdR(&info, id); std::printf("  preset %d pitch %.2f yaw %.2f zoom %.2f\n", info.id, info.pitch, info.yaw, info.zoom); return r; }) != 0) continue;
        d->aiGetGimbalPresetNameWithIdR(&name, id);
        std::string text(reinterpret_cast<const char *>(name.data_uint8), static_cast<size_t>(std::max(0, std::min(name.len, 64))));
        info.name_len = static_cast<int32_t>(text.size());
        std::memset(info.name, 0, sizeof info.name);
        std::memcpy(info.name, text.data(), text.size());
        step("r5_update_preset_" + std::to_string(id) + "_same", [&] { return d->aiUpdGimbalPresetR(&info, false); });
        step("r5_set_preset_name_" + std::to_string(id) + "_same", [&] { return d->aiSetGimbalPresetNameWithIdR(text, id); });
        last = info;
    }
    if (toggles && !preset_ids.empty()) {
        int32_t id = last.id;
        step("r5_delete_preset_" + std::to_string(id), [&] { return d->aiDelGimbalPresetR(id); });
        step("r5_add_preset_" + std::to_string(id) + "_back", [&] { return d->aiAddGimbalPresetR(&last); });
        step("r5_preset_list_after", [&] { Device::DevDataArray again{}; int r = d->aiGetGimbalPresetListR(&again); std::printf("  %d preset(s) after\n", again.len); return r; });
        finally("Saved position " + std::to_string(id) + ": deleted and added back with the same place, zoom and name");
    }
}

// ════════════ The microphone switch, as the SDK sends it ════════════
static void run_mic(Device *d) {
    load_hidden_symbols();
    const char *mic = "Device::cameraSetAudioMicDisabledU(bool)";
    step("mic_status_before", [&] { Device::CameraStatus s{}; int r = d->cameraGetCameraStatusU(s); hexdump("CameraStatus", &s, sizeof s); return r; });
    step("mic_disabled_true", [&] { return hidden(d, mic, true); }, 1500);
    step("mic_status_disabled", [&] { Device::CameraStatus s{}; int r = d->cameraGetCameraStatusU(s); hexdump("CameraStatus", &s, sizeof s); return r; }, 3000);
    step("mic_disabled_false", [&] { return hidden(d, mic, false); }, 1500);
    step("mic_status_enabled", [&] { Device::CameraStatus s{}; int r = d->cameraGetCameraStatusU(s); hexdump("CameraStatus", &s, sizeof s); return r; }, 3000);
    finally("Microphone: switched on (OBSBOT's own command, disabled = false)");
}

static void finish() {
    pause(1500);
    std::printf("PROBE END %.6f\n", now());
    if (!final_states.empty()) {
        std::printf("FINAL STATES (set these back in the app if they differ from what you had):\n");
        for (auto &s : final_states) std::printf("  - %s\n", s.c_str());
    }
    std::fflush(stdout);
    Devices::get().close();
}

int main(int argc, char **argv) {
    bool toggles = false, moves = false, hidden_only = false, round4 = false, round5 = false, images = false, mic_only = false;
    for (int i = 1; i < argc; ++i) {
        if (std::strcmp(argv[i], "--hidden") == 0) hidden_only = true;
        if (std::strcmp(argv[i], "--round4") == 0) round4 = true;
        if (std::strcmp(argv[i], "--round5") == 0) round5 = true;
        if (std::strcmp(argv[i], "--mic") == 0) mic_only = true;
        if (std::strcmp(argv[i], "--sleep-images") == 0) images = true;
        if (std::strcmp(argv[i], "--toggles") == 0) toggles = true;
        if (std::strcmp(argv[i], "--moves") == 0 || std::strcmp(argv[i], "--with-moves") == 0) moves = true;
    }
    std::printf("PROBE START %.6f toggles=%d moves=%d hidden=%d\n", now(), toggles, moves, hidden_only);
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

    if (mic_only) {
        run_mic(dev.get());
        finish();
        return 0;
    }
    if (round5) {
        run_round5(dev.get(), toggles);
        finish();
        return 0;
    }
    if (round4) {
        run_round4(dev.get(), moves, images);
        finish();
        return 0;
    }
    if (hidden_only) {
        run_hidden(dev.get(), toggles, moves);
        finish();
        return 0;
    }

    // ════════════ Reads ════════════
    Device::CameraStatus status{};
    step("get_camera_status", [&] { int r = dev->cameraGetCameraStatusU(status); hexdump("CameraStatus", &status, sizeof status); return r; });
    const unsigned char *raw = reinterpret_cast<const unsigned char *>(&status);
    // Bytes this project has verified in the status reply (Assistant/reference): 0x01 target selection gesture,
    // 0x06 HDR, 0x07 face exposure priority, 0x18/0x1c AI mode and sub-mode.
    const bool target_gesture = raw[0x01] == 1, hdr = raw[0x06] == 1, face_ae = raw[0x07] == 1;
    std::printf("  status: target gesture %d, HDR %d, face exposure %d, AI mode %d/%d\n", target_gesture, hdr, face_ae, raw[0x18], raw[0x1c]);

    step("get_ai_status", [&] { Device::AiStatus s{}; int r = dev->aiGetAiStatusR(&s); hexdump("AiStatus", &s, sizeof s); return r; });
    step("get_gimbal_state", [&] { Device::AiGimbalStateInfo s{}; int r = dev->aiGetGimbalStateR(&s); hexdump("AiGimbalStateInfo", &s, sizeof s); return r; });
    float att[3] = {0, 0, 0};
    step("get_gimbal_attitude", [&] { int r = dev->gimbalGetAttitudeInfoR(att); std::printf("  attitude roll %.2f pitch %.2f yaw %.2f\n", att[0], att[1], att[2]); return r; });
    float zoom = 1.0f;
    step("get_zoom", [&] { int r = dev->cameraGetZoomAbsoluteR(zoom); std::printf("  zoom %.3f\n", zoom); return r; });
    step("get_zoom_range", [&] { Device::UvcParamRange r{}; int ret = dev->cameraGetRangeZoomAbsoluteR(r); range("zoom", r); return ret; });
    step("get_boot_position", [&] { Device::PresetPosInfo p{}; int r = dev->aiGetGimbalBootPosR(&p); std::printf("  boot id %d pitch %.2f yaw %.2f zoom %.2f\n", p.id, p.pitch, p.yaw, p.zoom); return r; });

    Device::DevDataArray presets{};
    step("get_preset_list", [&] { int r = dev->aiGetGimbalPresetListR(&presets); hexdump("DevDataArray", &presets, sizeof presets); return r; });
    std::vector<int> preset_ids;
    for (int i = 0; i < presets.len && i < 16; ++i) preset_ids.push_back(presets.data_int32[i]);
    for (int id : preset_ids) {
        step("get_preset_info_" + std::to_string(id), [&] { Device::PresetPosInfo p{}; int r = dev->aiGetGimbalPresetInfoWithIdR(&p, id); std::printf("  preset %d pitch %.2f yaw %.2f zoom %.2f\n", p.id, p.pitch, p.yaw, p.zoom); return r; });
        step("get_preset_name_" + std::to_string(id), [&] { Device::DevDataArray n{}; int r = dev->aiGetGimbalPresetNameWithIdR(&n, id); std::printf("  name length %d\n", n.len); return r; });
    }
    Device::DevDataArray zones{};
    step("get_zone_preset_list", [&] { int r = dev->aiGetZonePresetListR(&zones); hexdump("DevDataArray", &zones, sizeof zones); return r; });
    for (int i = 0; i < zones.len && i < 16; ++i) {
        int id = zones.data_int32[i];
        step("get_zone_preset_info_" + std::to_string(id), [&] { Device::PresetPosInfo p{}; return dev->aiGetZonePresetInfoWithIdR(&p, id); });
        step("get_zone_preset_name_" + std::to_string(id), [&] { Device::DevDataArray n{}; return dev->aiGetZonePresetNameWithIdR(&n, id); });
    }

    bool zone_auto = false;
    float yaw_min = 0, yaw_max = 0, pitch_min = 0, pitch_max = 0;
    int ok_auto = step("get_limited_zone_auto_select", [&] { int r = dev->aiGetLimitedZoneTrackAutoSelectR(zone_auto); std::printf("  auto select %d\n", zone_auto); return r; });
    int ok_ymin = step("get_limited_zone_yaw_min", [&] { int r = dev->aiGetLimitedZoneTrackYawMinR(yaw_min, 0); std::printf("  yaw min %.2f\n", yaw_min); return r; });
    int ok_ymax = step("get_limited_zone_yaw_max", [&] { int r = dev->aiGetLimitedZoneTrackYawMaxR(yaw_max, 0); std::printf("  yaw max %.2f\n", yaw_max); return r; });
    int ok_pmin = step("get_limited_zone_pitch_min", [&] { int r = dev->aiGetLimitedZoneTrackPitchMinR(pitch_min, 0); std::printf("  pitch min %.2f\n", pitch_min); return r; });
    int ok_pmax = step("get_limited_zone_pitch_max", [&] { int r = dev->aiGetLimitedZoneTrackPitchMaxR(pitch_max, 0); std::printf("  pitch max %.2f\n", pitch_max); return r; });
    step("get_limited_zone_yaw_min_default", [&] { float d = 0; int r = dev->aiGetLimitedZoneTrackYawMinR(d, -1); std::printf("  default yaw min %.2f\n", d); return r; });
    step("get_limited_zone_yaw_max_default", [&] { float d = 0; int r = dev->aiGetLimitedZoneTrackYawMaxR(d, -1); std::printf("  default yaw max %.2f\n", d); return r; });
    step("get_limited_zone_pitch_min_default", [&] { float d = 0; int r = dev->aiGetLimitedZoneTrackPitchMinR(d, -1); std::printf("  default pitch min %.2f\n", d); return r; });
    step("get_limited_zone_pitch_max_default", [&] { float d = 0; int r = dev->aiGetLimitedZoneTrackPitchMaxR(d, -1); std::printf("  default pitch max %.2f\n", d); return r; });
    step("get_limited_zone_init_pos", [&] { Device::PresetPosInfo p{}; int r = dev->aiGetLimitedZoneTrackInitPosR(&p, 0); std::printf("  init pitch %.2f yaw %.2f zoom %.2f\n", p.pitch, p.yaw, p.zoom); return r; });

    // Image and UVC-style settings through the SDK (these may use XU commands rather than standard controls).
    struct Pic { const char *name; std::function<int(int32_t &)> get; std::function<int(int32_t)> set; std::function<int(Device::UvcParamRange &)> rng; };
    std::vector<Pic> pics = {
        {"brightness", [&](int32_t &v) { return dev->cameraGetImageBrightnessR(v); }, [&](int32_t v) { return dev->cameraSetImageBrightnessR(v); }, [&](Device::UvcParamRange &r) { return dev->cameraGetRangeImageBrightnessR(r); }},
        {"contrast", [&](int32_t &v) { return dev->cameraGetImageContrastR(v); }, [&](int32_t v) { return dev->cameraSetImageContrastR(v); }, [&](Device::UvcParamRange &r) { return dev->cameraGetRangeImageContrastR(r); }},
        {"hue", [&](int32_t &v) { return dev->cameraGetImageHueR(v); }, [&](int32_t v) { return dev->cameraSetImageHueR(v); }, [&](Device::UvcParamRange &r) { return dev->cameraGetRangeImageHueR(r); }},
        {"saturation", [&](int32_t &v) { return dev->cameraGetImageSaturationR(v); }, [&](int32_t v) { return dev->cameraSetImageSaturationR(v); }, [&](Device::UvcParamRange &r) { return dev->cameraGetRangeImageSaturationR(r); }},
        {"sharpness", [&](int32_t &v) { return dev->cameraGetImageSharpR(v); }, [&](int32_t v) { return dev->cameraSetImageSharpR(v); }, [&](Device::UvcParamRange &r) { return dev->cameraGetRangeImageSharpR(r); }},
    };
    for (auto &pic : pics) {
        int32_t value = 0;
        Device::UvcParamRange r{};
        step(std::string("get_") + pic.name + "_range", [&] { int ret = pic.rng(r); range(pic.name, r); return ret; });
        int got = step(std::string("get_") + pic.name, [&] { int ret = pic.get(value); std::printf("  %s %d\n", pic.name, value); return ret; });
        if (got == 0) {
            int32_t other = value + 1 <= r.max_ || r.max_ == 0 ? value + 1 : value - 1;
            step(std::string("set_") + pic.name + "_" + std::to_string(other), [&] { return pic.set(other); });
            step(std::string("set_") + pic.name + "_back_" + std::to_string(value), [&] { return pic.set(value); });
        }
    }
    {
        Device::DevWhiteBalanceType wb{}; int32_t param = 0;
        int got = step("get_white_balance", [&] { int r = dev->cameraGetWhiteBalanceR(wb, param); std::printf("  white balance type %d param %d\n", static_cast<int>(wb), param); return r; });
        step("get_white_balance_list", [&] { std::vector<int32_t> list; int32_t lo = 0, hi = 0; int r = dev->cameraGetWhiteBalanceListR(list, lo, hi); std::printf("  %zu types, %d..%d\n", list.size(), lo, hi); return r; });
        step("get_white_balance_range", [&] { Device::UvcParamRange r{}; int ret = dev->cameraGetRangeWhiteBalanceR(r); range("white balance", r); return ret; });
        if (got == 0) step("set_white_balance_same", [&] { return dev->cameraSetWhiteBalanceR(wb, param); });
    }
    {
        int32_t freq = 0;
        int got = step("get_anti_flicker", [&] { int r = dev->cameraGetAntiFlickR(freq); std::printf("  anti-flicker %d\n", freq); return r; });
        step("get_anti_flicker_range", [&] { Device::UvcParamRange r{}; int ret = dev->cameraGetRangeAntiFlickR(r); range("anti-flicker", r); return ret; });
        if (got == 0) step("set_anti_flicker_same", [&] { return dev->cameraSetAntiFlickR(freq); });
    }
    {
        int32_t shutter = 0; bool auto_exposure = true;
        int got = step("get_exposure", [&] { int r = dev->cameraGetExposureAbsolute(shutter, auto_exposure); std::printf("  shutter %d auto %d\n", shutter, auto_exposure); return r; });
        step("get_exposure_range", [&] { Device::UvcParamRange r{}; int ret = dev->cameraGetRangeExposureAbsolute(r); range("exposure", r); return ret; });
        if (got == 0) step("set_exposure_same", [&] { return dev->cameraSetExposureAbsolute(shutter, auto_exposure); });
    }
    {
        int32_t focus = 0; bool auto_focus = true;
        int got = step("get_focus", [&] { int r = dev->cameraGetFocusAbsolute(focus, auto_focus); std::printf("  focus %d auto %d\n", focus, auto_focus); return r; });
        step("get_focus_range", [&] { Device::UvcParamRange r{}; int ret = dev->cameraGetRangeFocusAbsolute(r); range("focus", r); return ret; });
        if (got == 0) step("set_focus_same", [&] { return dev->cameraSetFocusAbsolute(focus, auto_focus); });
    }
    step("set_zoom_same", [&] { return dev->cameraSetZoomAbsoluteR(zoom); });

    // ════════════ Changes restored from values read above ════════════
    if (ok_ymin == 0) { step("set_limited_zone_yaw_min_-30", [&] { return dev->aiSetLimitedZoneTrackYawMinR(-30.0f); }); step("set_limited_zone_yaw_min_back", [&] { return dev->aiSetLimitedZoneTrackYawMinR(yaw_min); }); }
    if (ok_ymax == 0) { step("set_limited_zone_yaw_max_30", [&] { return dev->aiSetLimitedZoneTrackYawMaxR(30.0f); }); step("set_limited_zone_yaw_max_back", [&] { return dev->aiSetLimitedZoneTrackYawMaxR(yaw_max); }); }
    if (ok_pmin == 0) { step("set_limited_zone_pitch_min_-10", [&] { return dev->aiSetLimitedZoneTrackPitchMinR(-10.0f); }); step("set_limited_zone_pitch_min_back", [&] { return dev->aiSetLimitedZoneTrackPitchMinR(pitch_min); }); }
    if (ok_pmax == 0) { step("set_limited_zone_pitch_max_10", [&] { return dev->aiSetLimitedZoneTrackPitchMaxR(10.0f); }); step("set_limited_zone_pitch_max_back", [&] { return dev->aiSetLimitedZoneTrackPitchMaxR(pitch_max); }); }
    if (ok_auto == 0) { step("set_limited_zone_auto_select_flip", [&] { return dev->aiSetLimitedZoneTrackAutoSelectR(!zone_auto); }); step("set_limited_zone_auto_select_back", [&] { return dev->aiSetLimitedZoneTrackAutoSelectR(zone_auto); }); }
    step("set_face_exposure_flip", [&] { return dev->cameraSetFaceAER(face_ae ? 0 : 1); });
    step("set_face_exposure_back", [&] { return dev->cameraSetFaceAER(face_ae ? 1 : 0); });
    step("set_gesture_target_flip", [&] { return dev->aiSetGestureCtrlIndividualR(0, !target_gesture); });
    step("set_gesture_target_back", [&] { return dev->aiSetGestureCtrlIndividualR(0, target_gesture); });
    // HDR switching is slow: OBSBOT's guidance is 3 s between switches.
    step("set_hdr_flip", [&] { return dev->cameraSetWdrR(hdr ? Device::DevWdrModeNone : Device::DevWdrModeDol2TO1); });
    step("set_hdr_back", [&] { return dev->cameraSetWdrR(hdr ? Device::DevWdrModeDol2TO1 : Device::DevWdrModeNone); }, 3500);
    pause(3500);

    // ════════════ Toggles whose state cannot be read ════════════
    if (toggles) {
        step("set_face_focus_on", [&] { return dev->cameraSetFaceFocusR(true); });
        step("set_face_focus_off", [&] { return dev->cameraSetFaceFocusR(false); });
        step("set_face_focus_on_again", [&] { return dev->cameraSetFaceFocusR(true); });
        finally("Autofocus priority: Face");
        step("set_mirror_on", [&] { return dev->cameraSetImageFlipHorizonU(1); });
        step("set_mirror_off", [&] { return dev->cameraSetImageFlipHorizonU(0); });
        finally("Mirror image: off");
        step("set_mic_during_sleep_off", [&] { return dev->cameraSetMicrophoneDuringSleepU(0); });
        step("set_mic_during_sleep_on", [&] { return dev->cameraSetMicrophoneDuringSleepU(1); });
        finally("Listen while asleep: on");
        step("set_audio_auto_gain_on", [&] { return dev->cameraSetAudioAutoGainU(true); });
        step("set_audio_auto_gain_off", [&] { return dev->cameraSetAudioAutoGainU(false); });
        finally("Microphone auto gain: off");
        for (int g = 1; g <= 3; ++g) {
            step("set_gesture_" + std::to_string(g) + "_on", [&] { return dev->aiSetGestureCtrlIndividualR(g, true); });
            step("set_gesture_" + std::to_string(g) + "_off", [&] { return dev->aiSetGestureCtrlIndividualR(g, false); });
        }
        finally("Zoom gesture, Dynamic zoom, Reverse dynamic zoom: off");
        step("set_ai_auto_zoom_on", [&] { return dev->aiSetAiAutoZoomR(true); });
        step("set_ai_auto_zoom_off", [&] { return dev->aiSetAiAutoZoomR(false); });
        finally("AI auto zoom: off");
        step("set_yaw_reverse_on", [&] { return dev->aiSetGimbalYawDirReverseR(true); });
        step("set_yaw_reverse_off", [&] { return dev->aiSetGimbalYawDirReverseR(false); });
        finally("Pan direction reverse (camera): off");
        step("set_limited_zone_enabled_on", [&] { return dev->aiSetLimitedZoneTrackEnabledR(true); });
        step("set_limited_zone_enabled_off", [&] { return dev->aiSetLimitedZoneTrackEnabledR(false); });
        finally("Limited-zone tracking: off");
        step("set_vertical_track_headroom", [&] { return dev->aiSetTrackingModeR(Device::AiVTrackHeadroom); });
        step("set_vertical_track_motion", [&] { return dev->aiSetTrackingModeR(Device::AiVTrackMotion); });
        step("set_vertical_track_standard", [&] { return dev->aiSetTrackingModeR(Device::AiVTrackStandard); });
        finally("Vertical tracking: standard");
        step("set_camera_sleep_timer_600", [&] { return dev->cameraSetSuspendTimeU(600); });
        step("set_camera_sleep_timer_off", [&] { return dev->cameraSetSuspendTimeU(0); });
        finally("Camera's own sleep timer: off (the app's background service does the sleep timer)");
        for (int c = Device::AudioCtrlHiTiny; c <= Device::AudioCtrlPreset; ++c) {
            step("set_voice_command_" + std::to_string(c) + "_off", [&] { return dev->cameraSetAudioCtrlStateU(static_cast<Device::AudioCtrlCmdType>(c), 0); });
            step("set_voice_command_" + std::to_string(c) + "_on", [&] { return dev->cameraSetAudioCtrlStateU(static_cast<Device::AudioCtrlCmdType>(c), 1); });
        }
        finally("Voice commands: all on");
        step("set_led_pattern_on", [&] { return dev->cameraSetLedCtrlU(true); });
        step("set_led_pattern_off", [&] { return dev->cameraSetLedCtrlU(false); });
        finally("Zone-tracking LED pattern: off");
    }

    // ════════════ Moves ════════════
    if (moves) {
        const float pitch = att[1], yaw = att[2];
        auto back = [&](const char *name) { step(name, [&] { return dev->aiSetGimbalMotorAngleR(pitch, yaw); }); pause(2500); };
        step("move_ai_speed_pan_10", [&] { return dev->aiSetGimbalSpeedCtrlR(0.0, 10.0); });
        pause(800);
        step("move_ai_stop", [&] { return dev->aiSetGimbalStop(); });
        back("move_back_1");
        step("move_gimbal_speed_pan_10", [&] { return dev->gimbalSpeedCtrlR(0.0, 10.0); });
        pause(800);
        step("move_gimbal_speed_zero", [&] { return dev->gimbalSpeedCtrlR(0.0, 0.0); });
        back("move_back_2");
        step("move_motor_angle_pan_plus_10", [&] { return dev->aiSetGimbalMotorAngleR(pitch, yaw + 10.0f); });
        pause(2500);
        back("move_back_3");
        step("move_speed_position_pan_plus_10", [&] { return dev->gimbalSetSpeedPositionR(0.0f, pitch, yaw + 10.0f, 0.0f, 20.0f, 20.0f); });
        pause(2500);
        back("move_back_4");
        step("move_reset_to_zero", [&] { return dev->gimbalRstPosR(); });
        pause(3000);
        back("move_back_5");
        step("move_to_boot_position", [&] { return dev->aiTrgGimbalBootPosR(false); });
        pause(3000);
        back("move_back_6");
        if (!preset_ids.empty()) {
            step("move_to_preset_" + std::to_string(preset_ids[0]), [&] { return dev->aiTrgGimbalPresetR(preset_ids[0]); });
            pause(3000);
            back("move_back_7");
        }
        step("move_to_limited_zone_init_pos", [&] { return dev->aiTrgLimitedZoneTrackInitPosR(); });
        pause(3000);
        back("move_back_8");
        step("zoom_to_2", [&] { return dev->cameraSetZoomAbsoluteR(2.0f); });
        pause(2000);
        step("zoom_back", [&] { return dev->cameraSetZoomAbsoluteR(zoom); });
        pause(2000);
        step("zoom_with_speed_150", [&] { return dev->cameraSetZoomWithSpeedAbsoluteR(150, 5); });
        pause(2500);
        step("zoom_with_speed_back", [&] { return dev->cameraSetZoomWithSpeedAbsoluteR(static_cast<uint32_t>(zoom * 100.0f + 0.5f), 5); });
        pause(2500);
        // AI tracking modes: each for a moment, ending on no tracking.
        struct Mode { const char *name; Device::AiWorkModeType mode; int sub; };
        Mode modes[] = {
            {"human_normal", Device::AiWorkModeHuman, Device::AiSubModeNormal},
            {"human_upper_body", Device::AiWorkModeHuman, Device::AiSubModeUpperBody},
            {"human_close_up", Device::AiWorkModeHuman, Device::AiSubModeCloseUp},
            {"human_head_hide", Device::AiWorkModeHuman, Device::AiSubModeHeadHide},
            {"human_lower_body", Device::AiWorkModeHuman, Device::AiSubModeLowerBody},
            {"group", Device::AiWorkModeGroup, 0},
            {"hand", Device::AiWorkModeHand, 0},
            {"whiteboard", Device::AiWorkModeWhiteBoard, 0},
            {"desk", Device::AiWorkModeDesk, 0},
            {"none", Device::AiWorkModeNone, 0},
        };
        for (auto &m : modes) {
            step(std::string("ai_mode_") + m.name, [&] { return dev->cameraSetAiModeU(m.mode, m.sub); }, 2500);
            step(std::string("ai_mode_") + m.name + "_status", [&] { Device::CameraStatus s{}; int r = dev->cameraGetCameraStatusU(s); const unsigned char *b = reinterpret_cast<const unsigned char *>(&s); std::printf("  status AI mode %d/%d\n", b[0x18], b[0x1c]); return r; }, 1500);
        }
        finally("AI tracking: no tracking");
        step("ai_enabled_off", [&] { return dev->aiSetEnabledR(false); });
        step("ai_enabled_on", [&] { return dev->aiSetEnabledR(true); });
        step("ai_track_mode_human_on", [&] { return dev->aiSetAiTrackModeEnabledR(Device::AiTrackHumanNormal, true); }, 2000);
        step("ai_track_mode_human_off", [&] { return dev->aiSetAiTrackModeEnabledR(Device::AiTrackHumanNormal, false); }, 2000);
        step("zone_tracking_on", [&] { return dev->aiSetZoneTrackStateR(true); }, 2000);
        step("zone_tracking_off", [&] { return dev->aiSetZoneTrackStateR(false); }, 2000);
        step("ai_mode_none_again", [&] { return dev->cameraSetAiModeU(Device::AiWorkModeNone, 0); }, 2000);
        back("move_back_9");
        step("sleep", [&] { return dev->cameraSetDevRunStatusR(Device::DevStatusSleep); }, 1500);
        pause(4000);
        step("wake", [&] { return dev->cameraSetDevRunStatusR(Device::DevStatusRun); }, 1500);
        pause(5000);
        back("move_back_10");
        finally("Camera awake, at the position it started from");
    }

    finish();
    return 0;
}
