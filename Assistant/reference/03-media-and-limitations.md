## 6. Preview, snapshots, recording and audio

The Domatix app uses GStreamer for live preview and snapshot capture,
with GTK4 paintable integration. The Qt app demonstrates a broader
desktop experience including preview and virtual-camera concepts.

The MCP project lists operations for: - Preview. - Snapshot. - Video
recording. - Audio capture/recording.

However, these capabilities are not all necessarily camera firmware
controls. Some are host-side media pipeline features. Keep the
architecture separate: - Camera control service. - Video capture/preview
pipeline. - Snapshot/recording service. - Optional audio capture
service.

Do not repeatedly open and close streaming sessions during early
hardware testing.

------------------------------------------------------------------------

## 7. Known Linux and firmware limitations

1.  **USB streaming fragility:** Tiny 2 Lite firmware 5.10 may
    disconnect with `-71 EPROTO` if clients rapidly start/stop streaming
    while XU writes or pan commands are interleaved.
2.  Use one long-lived stream during preview tests.
3.  Avoid repeated stream negotiation cycles.
4.  After unplug/replug, wait before starting the stream.
5.  A single preset save/pan/recall/capture round trip has been reported
    as okay.
6.  MCP Linux implementation reports cached/last-commanded gimbal
    position rather than live feedback.
7.  MCP Linux gimbal speed operation is unavailable.
8.  Face AE is not reliable through the standard V4L2 exposure route.
9.  Narrow FOV was a no-op on tested Tiny 2 Lite firmware 5.10.
10. Tracking speed enum may vary with firmware.
11. Some status fields are inherited from regular Tiny 2 mapping and
    remain unverified on Lite.

**Do not run aggressive integration tests.** Some scripts physically
move the gimbal. Review their behavior before running.

------------------------------------------------------------------------

