using System.Globalization;
using System.Text.Json;

namespace Loupedeck.PirateWavePlugin;

public sealed class PirateWavePlugin : Plugin
{
    internal static ControlClient Client { get; private set; } = new();
    public override bool UsesApplicationApiOnly => true;
    public override bool HasNoApplication => true;
    public PirateWavePlugin() { Client = new(); }
    public override void Load() { Client.Diagnostic += message => Log.Info(message); Log.Info("Starting PirateWave controller connection"); Client.Start(); }
    public override void Unload() => Client.Dispose();
}

// The final selector is serialized as structured JSON. Names may contain separators
// or Unicode, and never become unstable row/array indexes in saved profiles.
internal sealed class Binding
{
    private const string NodeControl = "node";
    private const string TargetControl = "target";
    private const string ActionControl = "action";
    private const string SensitivityControl = "sensitivity";
    private readonly ActionEditorAction owner;
    private readonly string category;
    private readonly bool adjustment;
    private readonly Action changed;
    private Entry[] previous = [];
    private bool? wasConnected;
    private string previousStatus = "";
    internal Binding(ActionEditorAction owner, string category, bool adjustment)
    {
        this.owner = owner; this.category = category; this.adjustment = adjustment;
        changed = Update;
        owner.ActionEditor.AddControlEx(new ActionEditorListbox(NodeControl, "Node / source", "Match by exact name in the current project").SetRequired());
        owner.ActionEditor.AddControlEx(new ActionEditorListbox(TargetControl, "Effect / slot / destination", "Choose a unique target").SetRequired());
        owner.ActionEditor.AddControlEx(new ActionEditorListbox(ActionControl, adjustment ? "Parameter" : "Action", "Select a control").SetRequired());
        if (adjustment) owner.ActionEditor.AddControlEx(new ActionEditorSlider(SensitivityControl, "Sensitivity", "Multiplier of the default step per detent").SetValues(0.01, 100, 1, 0.01).SetFormatString("{0}x"));
        owner.ActionEditor.ListboxItemsRequested += Populate;
        owner.ActionEditor.ControlValueChanged += (_, e) => {
            if (string.Equals(e.ControlName, NodeControl, StringComparison.OrdinalIgnoreCase)) {
                e.ActionEditorState.SetValue(TargetControl, ""); e.ActionEditorState.SetValue(ActionControl, "");
                owner.ActionEditor.ListboxItemsChanged(TargetControl); owner.ActionEditor.ListboxItemsChanged(ActionControl);
            } else if (string.Equals(e.ControlName, TargetControl, StringComparison.OrdinalIgnoreCase)) {
                e.ActionEditorState.SetValue(ActionControl, ""); owner.ActionEditor.ListboxItemsChanged(ActionControl);
            }
        };
    }
    internal Action? Refresh;
    internal void Load() { PirateWavePlugin.Client.Changed += changed; Update(); }
    internal void Unload() => PirateWavePlugin.Client.Changed -= changed;
    private void Update()
    {
        var client = PirateWavePlugin.Client;
        var entries = Entries;
        var connected = client.Connected;
        var status = client.Status;
        // A state subscription also contains rapidly changing values. Invalidating
        // dropdowns for every packet makes Logi repeatedly reload an open editor.
        var choicesChanged = wasConnected != connected || previous.Length != entries.Length ||
            previous.Where((entry, i) => i < entries.Length && (entry.Selector != entries[i].Selector || entry.Label != entries[i].Label)).Any();
        var feedbackChanged = choicesChanged || previousStatus != status ||
            previous.Where((entry, i) => i < entries.Length && (entry.State != entries[i].State || entry.Active != entries[i].Active || entry.Value != entries[i].Value || entry.Unit != entries[i].Unit || !entry.Choices.SequenceEqual(entries[i].Choices))).Any();
        previous = entries; wasConnected = connected; previousStatus = status;
        if (choicesChanged)
            foreach (var control in new[] { NodeControl, TargetControl, ActionControl }) owner.ActionEditor.ListboxItemsChanged(control);
        if (feedbackChanged) Refresh?.Invoke();
    }
    private Entry[] Entries => PirateWavePlugin.Client.Snapshot.Entries.Where(e => e.Selector.Category == category && e.Adjustment == adjustment).ToArray();
    private static string NodeKey(Entry e) => JsonSerializer.Serialize(new[] { e.Selector.NodeKind, e.Selector.Node }, ControlClient.Json);
    private static string TargetKey(Entry e) => JsonSerializer.Serialize(new[] { e.Selector.Target, e.Selector.Plugin }, ControlClient.Json);
    private void Populate(object? sender, ActionEditorListboxItemsRequestedEventArgs args)
    {
        var entries = Entries;
        owner.Plugin?.Log.Verbose($"PirateWave editor request: {category}/{args.ControlName}, {entries.Length} available controls, {PirateWavePlugin.Client.Status}");
        // Logi normalizes editor control identifiers to lowercase. Target/node
        // names inside selectors remain exact and case-sensitive project data.
        if (string.Equals(args.ControlName, NodeControl, StringComparison.OrdinalIgnoreCase))
        {
            if (!PirateWavePlugin.Client.Connected) {
                args.AddItem("__piratewave_offline__", "PirateWave offline — run PirateWave and enable external controllers", PirateWavePlugin.Client.Status);
                return;
            }
            if (entries.Length == 0) {
                args.AddItem("__piratewave_empty__", "No matching controls in the current project", "Add a suitable node, effect or sound in PirateWave, then return here.");
                return;
            }
            foreach (var group in entries.GroupBy(NodeKey))
                args.AddItem(group.Key, group.First().Selector.Node.Length == 0 ? (category == "application" ? "PirateWave" : "Soundboard") : $"{group.First().Selector.Node} ({group.First().Selector.NodeKind})", "Current project name match");
        }
        else if (string.Equals(args.ControlName, TargetControl, StringComparison.OrdinalIgnoreCase))
        {
            foreach (var group in entries.Where(e => NodeKey(e) == args.ActionEditorState.GetControlValue(NodeControl)).GroupBy(TargetKey))
                args.AddItem(group.Key, group.First().Selector.Target.Length == 0 ? "This node" : group.First().Selector.Target, "Select target");
        }
        else if (string.Equals(args.ControlName, ActionControl, StringComparison.OrdinalIgnoreCase))
        {
            foreach (var group in entries.Where(e => NodeKey(e) == args.ActionEditorState.GetControlValue(NodeControl) && TargetKey(e) == args.ActionEditorState.GetControlValue(TargetControl)).GroupBy(e => JsonSerializer.Serialize(e.Selector, ControlClient.Json)))
                args.AddItem(group.Key, group.First().Label + (group.Count() > 1 ? " — Ambiguous target" : ""), "Works without a keyboard shortcut");
        }
    }
    internal static Selector? Selected(ActionEditorActionParameters args)
    {
        try { return JsonSerializer.Deserialize<Selector>(args.GetString(ActionControl, ""), ControlClient.Json); }
        catch (JsonException) { return null; }
    }
    internal bool Run(ActionEditorActionParameters args, string op, int ticks = 0)
    {
        var selected = Selected(args);
        if (selected == null || selected.Category != category) return false;
        if (PirateWavePlugin.Client.Find(selected, out _) == null) return false;
        var sensitivity = double.TryParse(args.GetString(SensitivityControl, "1"), NumberStyles.Float, CultureInfo.InvariantCulture, out var value) ? value : 1;
        var result = PirateWavePlugin.Client.Submit(op, selected, ticks, sensitivity);
        Refresh?.Invoke(); return result;
    }
    internal string Text(ActionEditorActionParameters args)
    {
        var selected = Selected(args);
        if (selected == null) return PirateWavePlugin.Client.Connected ? "Configure PirateWave action" : "PirateWave offline\nRun PirateWave and enable external controllers";
        var e = PirateWavePlugin.Client.Find(selected, out var error);
        if (e == null) return $"{selected.Node}\n{error}";
        var status = PirateWavePlugin.Client.Status;
        if (status != "Connected" && status != "Accepted") return $"{e.Label}\n{status}";
        var detail = e.Adjustment ? (e.Choices.Length > 0 ? e.Choices[Math.Clamp((int)Math.Round(e.Value), 0, e.Choices.Length - 1)] : $"{e.Value:0.###} {e.Unit}") : e.State;
        if (e.Adjustment && selected.Category == "mixer" && selected.Action == "pan") detail = e.Value == 0 ? "Center" : $"{Math.Abs(e.Value) * 100:0.#}% {(e.Value < 0 ? "L" : "R")}";
        if (e.Adjustment && e.State.Length > 0) detail += "\n" + e.State;
        return $"{(selected.Target.Length > 0 ? selected.Target : selected.Node.Length > 0 ? selected.Node : "PirateWave")}\n{e.Label}\n{detail}";
    }
    internal bool? Indicator(ActionEditorActionParameters args, bool command)
    {
        var client = PirateWavePlugin.Client;
        if (client.Status is not ("Connected" or "Accepted")) return null;
        var selector = Selected(args);
        var entry = selector == null ? null : client.Find(selector, out _);
        return entry == null ? null : ControlFeedback.Active(entry, command);
    }
    internal BitmapImage Image(ActionEditorActionParameters args, int width, int height, bool command)
    {
        var active = Indicator(args, command);
        var text = Text(args);
        if (active is { } on) text += $" · {(on ? "ON" : "OFF")}";
        return Render(text, width, height, active);
    }
    internal static BitmapImage Render(string text, int width, int height, bool? active)
    {
        using var builder = new BitmapBuilder(width, height);
        builder.Clear(0xFF191A1F);
        var color = BitmapColor.FromArgb(ControlFeedback.Color(active));
        builder.DrawText(text, null, color, 12, 0, 0);
        if (active.HasValue) builder.FillRectangle(0, Math.Max(0, height - 4), width, Math.Min(4, height), color);
        return builder.ToImage();
    }
}

public abstract class PirateWaveCommand : ActionEditorCommand
{
    private readonly Binding binding;
    protected PirateWaveCommand(string name, string label, string category)
    {
        Name = name; DisplayName = label; GroupName = label;
        binding = new(this, category, false) { Refresh = ActionImageChanged };
    }
    protected override bool OnLoad() { binding.Load(); return true; }
    protected override bool OnUnload() { binding.Unload(); return true; }
    protected override bool RunCommand(ActionEditorActionParameters args) => binding.Run(args, "execute");
    protected override string GetCommandDisplayName(ActionEditorActionParameters args) => binding.Text(args);
    protected override BitmapImage GetCommandImage(ActionEditorActionParameters args, int width, int height) => binding.Image(args, width, height, true);
}
public sealed class ApplicationCommand() : PirateWaveCommand("PirateWaveApplication", "Application", "application");
public sealed class NodeCommand() : PirateWaveCommand("PirateWaveNode", "Nodes", "node");
public sealed class EffectCommand() : PirateWaveCommand("PirateWaveEffect", "Effects", "effect");
public sealed class SwitchCommand() : PirateWaveCommand("PirateWaveSwitch", "Switch outputs", "switch");
public sealed class SoundCommand() : PirateWaveCommand("PirateWaveSound", "Soundboard", "soundboard");
public sealed class RecordingCommand() : PirateWaveCommand("PirateWaveRecording", "Recording slots", "recording_slot");

public abstract class PirateWaveAdjustment : ActionEditorAdjustment
{
    private readonly Binding binding;
    protected PirateWaveAdjustment(string name, string label, string category) : base(true)
    {
        Name = name; DisplayName = label; GroupName = label; ResetCommandDisplayName = "Press: mute / center / reset";
        binding = new(this, category, true) { Refresh = () => { AdjustmentValueChanged(); ActionImageChanged(); } };
    }
    protected override bool OnLoad() { binding.Load(); return true; }
    protected override bool OnUnload() { binding.Unload(); return true; }
    protected override bool ApplyAdjustment(ActionEditorActionParameters args, int diff) => binding.Run(args, "adjust", diff);
    protected override bool RunCommand(ActionEditorActionParameters args) => binding.Run(args, "reset");
    protected override string GetCommandDisplayName(ActionEditorActionParameters args) => binding.Text(args);
    protected override string GetAdjustmentDisplayName(ActionEditorActionParameters args) => binding.Text(args);
    protected override BitmapImage GetAdjustmentImage(ActionEditorActionParameters args, int width, int height) => binding.Image(args, width, height, false);
    protected override BitmapImage GetCommandImage(ActionEditorActionParameters args, int width, int height) => binding.Image(args, width, height, true);
}
public sealed class MixerAdjustment() : PirateWaveAdjustment("PirateWaveMixer", "Volume & pan", "mixer");
public sealed class EffectAdjustment() : PirateWaveAdjustment("PirateWaveEffectParameter", "Effect parameters", "effect_parameter");
public sealed class VstAdjustment() : PirateWaveAdjustment("PirateWaveVstParameter", "VST parameters", "vst_parameter");

public sealed class DuckingAdjustment() : PirateWaveAdjustment("PirateWaveDuckingParameter", "Auto Ducking parameters", "ducking_parameter");

public sealed class ExtensionCommand() : PirateWaveCommand("PirateWaveExtensionAction", "Extension actions", "extension_action");
public sealed class ExtensionAdjustment() : PirateWaveAdjustment("PirateWaveExtensionParameter", "Extension node parameters", "extension_parameter");
