namespace Loupedeck.PirateWavePlugin;

internal static class ControlFeedback
{
    internal const uint OnColor = 0xFF4ADE80;
    internal const uint OffColor = 0xFFF87171;
    internal const uint NeutralColor = 0xFFE5E7EB;
    internal static uint Color(bool? active) => active switch { true => OnColor, false => OffColor, _ => NeutralColor };

    internal static bool? Active(Entry entry, bool command)
    {
        // Volume's press toggles mute; rotating it still displays neutral gain.
        if (entry.Adjustment && !(command && entry.Selector.Category == "mixer" && entry.Selector.Action == "volume")) return null;
        if (entry.Active is { } active) return active;

        // Older PirateWave builds omit the optional typed state. Only recognize known
        // action/state pairs; never guess a boolean from an arbitrary label.
        var pair = (entry.Selector.Category, entry.Selector.Action);
        return pair switch {
            ("application", "start_audio" or "stop_audio" or "toggle_audio") => Match(entry.State, "Running", "Stopped"),
            ("node", "mute" or "unmute" or "toggle_mute") => entry.State == "Muted" ? true : entry.State is "Unmuted" or "Ready" or "Solo" ? false : null,
            ("node", "solo_on" or "solo_off" or "toggle_solo") => Match(entry.State, "Solo on", "Solo off"),
            ("node", "listen_on" or "listen_off" or "toggle_listening") => Match(entry.State, "Listening", "Not listening"),
            ("node", "enable" or "disable" or "toggle_disabled" or "enable_ducking" or "disable_ducking" or "toggle_ducking") => entry.State == "Enabled" ? true : entry.State is "Bypassed" or "Disabled" ? false : null,
            ("node", "start_recording" or "stop_recording") => Match(entry.State, "Armed", "Stopped"),
            ("effect", "enable" or "disable" or "toggle") => Match(entry.State, "Enabled", "Disabled"),
            ("switch", "activate") => Match(entry.State, "Active", "Inactive"),
            ("mixer", "volume") => entry.State == "Muted" ? true : entry.State.Length == 0 ? false : null,
            _ => null,
        };
    }
    private static bool? Match(string state, string on, string off) => state == on ? true : state == off ? false : null;
}
