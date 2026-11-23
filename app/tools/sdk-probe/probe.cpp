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
// Never: factory reset; adding, deleting, renaming or updating presets or zone presets; setting or resetting the
// start-up position or the zone start position; background images; the boot AI mode; privacy mode; firmware.

#include "dev/devs.hpp"
#include "dev/dev.hpp"

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

int main(int argc, char **argv) {
    bool toggles = false, moves = false;
    for (int i = 1; i < argc; ++i) {
        if (std::strcmp(argv[i], "--toggles") == 0) toggles = true;
        if (std::strcmp(argv[i], "--moves") == 0 || std::strcmp(argv[i], "--with-moves") == 0) moves = true;
    }
    std::printf("PROBE START %.6f toggles=%d moves=%d\n", now(), toggles, moves);
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

    pause(1500);
    std::printf("PROBE END %.6f\n", now());
    if (!final_states.empty()) {
        std::printf("FINAL STATES (set these back in the app if they differ from what you had):\n");
        for (auto &s : final_states) std::printf("  - %s\n", s.c_str());
    }
    std::fflush(stdout);
    Devices::get().close();
    return 0;
}
