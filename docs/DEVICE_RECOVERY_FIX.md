# Missing audio endpoint recovery fix (2026-10-24)

## Confirmed failure

A disconnected monitor output could stall all PirateWave processing, even while the
selected microphone continued to work in another application. In the reported
Virtual Desktop / SteamVR session, the missing output was WH-1000XM6 Stereo and
the intended microphone-to-Line-1 route remained configured and unmuted.

Logs showed continuous output underruns and dropped input frames after the
monitor disappeared. Disabling that output and restarting audio restored the
microphone; the counters then stopped increasing. The user confirmed recovery
with SteamVR running. This establishes the missing-output retry path as the
trigger, rather than microphone exclusive access.

## Cause and fix

Recovery previously called device enumeration on the render worker every 500 ms.
CPAL enumerates devices by probing their supported formats, which can involve
slow driver calls. The recovery timer was recorded before the attempt: an attempt
lasting longer than 500 ms caused the next loop to immediately retry. Audio
callbacks continued, but the graph could not keep up with them.

Device discovery now runs on a separate worker and publishes metadata through a
bounded channel. Recovery opens a configured device only when the published
inventory contains it; missing devices cannot cause repeated render-worker
enumerations. Secondary listening outputs use the same inventory, including its
default-output identity. The retry interval starts after recovery completes.
Shutdown signals the discovery worker without joining a slow driver operation.
No stream is transferred across threads; initial setup and actual reopening of
an endpoint still happen on the audio worker. A newly available device opening
can therefore still cause a brief pause; this change specifically removes the
continuous starvation caused by missing endpoints, not every driver stall.

## Validation

Tests cover input/output direction and missing-endpoint eligibility, nonblocking
polling/shutdown while discovery is deliberately stalled, existing device error
recovery, and soundboard continuity with missing devices. Workspace tests,
formatting and Clippy are run before delivering the release executable.

Historical workaround for the affected build (not needed for the current retry path): keep an unavailable Monitor output set to Disabled while
using Virtual Desktop. The fixed build can retain that output selection while it
is absent and discover it again when it returns. Physical reconnect qualification
of the new build remains separate from the confirmed workaround.
