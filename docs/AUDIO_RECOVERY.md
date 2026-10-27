# Audio device and application capture recovery

Output bus device assignments remain in the workspace even when the endpoint disappears. The Mixer displays the saved name with an unavailable suffix. While audio is running, the existing asynchronous discovery worker refreshes the Mixer list automatically; refreshing the list does not change assignments or restart the graph.

Every 500 ms, the engine checks stream errors, callback progress, and the latest device snapshot. It drops an affected output stream before reopening it, waits for unavailable devices to return, and retries failed opens. System-default output buses also follow default endpoint changes; explicitly selected devices remain selected. Other streams and the processing graph stay intact. Recovery cannot produce audio while Windows reports a device unavailable, and reopening includes the normal stream buffering delay.

Application Capture shares one Windows process-loopback client per process. Windows API failures trigger client replacement on its dedicated capture worker. A watchdog also probes a client after three seconds without packets. Because Windows can continue returning silent packets from a stale process graph, 30 seconds of uninterrupted digital silence also permits a health probe. This is a precaution, not a claim that silence means failure. Repeated idle probes back off to at most one per minute; nonzero audio resets the backoff. The first probe after an extended silent period can briefly overlap playback resuming. Finished capture workers are detected by the existing one-second application scan and recreated. Applications themselves are never restarted.

All old capture clients and event handles are released before replacement. Subscriber queues, error queues and discovery delivery remain bounded. Device lists are shared through existing status messages and cloned by the UI only when changed. No new discovery, locks, logging or allocations are added to the audio render path. The previous CPAL discovery leak fix is included; see [Device discovery memory](DEVICE_DISCOVERY_MEMORY.md).

## Validation

Validation snapshot from the recovery implementation on 2026-10-26 (before subsequent SDK/branding changes): 255 workspace tests passed, formatting and Clippy passed, both opt-in native recovery tests passed, and all 20 rack processor benchmarks reported zero processing allocations. The original Spotify failure and physical unplug/replug were not reproduced during this run.

- Deterministic tests cover missing packet detection, silent-packet probing, active audio avoiding probes, bounded retry intervals and reset after signal returns.
- Device tests cover retained explicit selections, disappearance/reappearance and system-default changes.
- Opt-in native tests inject errors into privately owned input/output and application capture streams, check resumed capture/playback and preservation of the graph, and exercise shared capture subscribers.
- Physical unplug/replug behavior and the user's original Spotify stall still depend on the Windows driver/application; fault injection does not reproduce every driver failure.
