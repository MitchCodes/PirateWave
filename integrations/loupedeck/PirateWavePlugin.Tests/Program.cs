using System.Diagnostics;
using System.Reflection;
using System.Runtime.Loader;
using System.Text.Json;
using System.IO.Pipes;
using System.Security.Principal;
using Loupedeck.PirateWavePlugin;

// Resolve SDK dependencies from the installed host or the verified CI SDK.
AssemblyLoadContext.Default.Resolving += (_, name) => {
    var sdkDir = Environment.GetEnvironmentVariable("PIRATEWAVE_LOUPEDECK_SDK_DIR") ?? Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles), "Logi", "LogiPluginService");
    var path = Path.Combine(sdkDir, name.Name + ".dll");
    return File.Exists(path) ? AssemblyLoadContext.Default.LoadFromAssemblyPath(path) : null;
};
if (args is ["--check-installation"]) {
    Tests.InstallationChecks();
    Tests.EditorChecks();
    Tests.ColorChecks();
    Console.WriteLine("PASS: Logi loader entry points, offline installation lifecycle and SDK dependent-dropdown/refresh checks");
} else {
    await Tests.Run(args[0]);
}

static class Tests
{
    static readonly string TestId = Guid.NewGuid().ToString("N");
    static string PipeName => "PirateWave.Control.v1." + WindowsIdentity.GetCurrent().User!.Value + ".test." + TestId;
    static void Check(bool condition, string message) { if (!condition) throw new Exception(message); }
    static async Task Until(Func<bool> predicate, string message) {
        var clock = Stopwatch.StartNew(); while (!predicate()) { if (clock.Elapsed > TimeSpan.FromSeconds(8)) throw new Exception(message); await Task.Delay(20); }
    }
    static Process Host(string exe) {
        var start = new ProcessStartInfo(exe, "--controller-test-host") { UseShellExecute = false, CreateNoWindow = true };
        start.Environment["PIRATEWAVE_CONTROLLER_TEST_ID"] = TestId;
        return Process.Start(start)!;
    }
    static async Task RawVersionCheck() {
        using var pipe = new NamedPipeClientStream(".", PipeName, PipeDirection.InOut, PipeOptions.Asynchronous);
        using var timeout = new CancellationTokenSource(3000); await pipe.ConnectAsync(timeout.Token);
        var bytes = JsonSerializer.SerializeToUtf8Bytes(new { id = 1, op = "hello", version = 999 });
        await pipe.WriteAsync(BitConverter.GetBytes(bytes.Length), timeout.Token); await pipe.WriteAsync(bytes, timeout.Token);
        var size = new byte[4]; await pipe.ReadExactlyAsync(size, timeout.Token);
        var response = new byte[BitConverter.ToInt32(size)]; await pipe.ReadExactlyAsync(response, timeout.Token);
        Check(!JsonDocument.Parse(response).RootElement.GetProperty("ok").GetBoolean(), "Wrong version accepted");
    }
    static async Task OversizeFrameCheck() {
        using var pipe = new NamedPipeClientStream(".", PipeName, PipeDirection.InOut, PipeOptions.Asynchronous);
        using var timeout = new CancellationTokenSource(3000); await pipe.ConnectAsync(timeout.Token);
        await pipe.WriteAsync(BitConverter.GetBytes(16 * 1024 * 1024 + 1), timeout.Token);
        Check(await pipe.ReadAsync(new byte[1], timeout.Token) == 0, "Oversized frame not rejected");
    }
    static bool SdkRun(Loupedeck.ActionEditorAction action, ControlClient client, Entry entry, string method, int ticks = 0) {
        typeof(PirateWavePlugin).GetProperty("Client", BindingFlags.Static | BindingFlags.NonPublic)!.SetValue(null, client);
        var binding = action.GetType().BaseType!.GetField("binding", BindingFlags.NonPublic | BindingFlags.Instance)!.GetValue(action)!;
        binding.GetType().GetField("Refresh", BindingFlags.NonPublic | BindingFlags.Instance)!.SetValue(binding, (Action)(() => { }));
        var parameters = new Loupedeck.ActionEditorActionParameters(new Dictionary<string,string> {
            ["Action"] = JsonSerializer.Serialize(entry.Selector, ControlClient.Json), ["Sensitivity"] = "1"
        });
        var callback = action.GetType().GetMethod(method, BindingFlags.NonPublic | BindingFlags.Instance)!;
        return (bool)callback.Invoke(action, method == "ApplyAdjustment" ? [parameters, ticks] : [parameters])!;
    }
    static void QueueChecks() {
        using var client = new ControlClient();
        typeof(ControlClient).GetField("connected", BindingFlags.NonPublic | BindingFlags.Instance)!.SetValue(client, true);
        typeof(ControlClient).GetField("snapshot", BindingFlags.NonPublic | BindingFlags.Instance)!.SetValue(client, new Snapshot(42, "Queue", [], ""));
        var selector = new Selector("mixer", "input", "Mic", "", "volume", "");
        client.Submit("adjust", selector, 1); client.Submit("adjust", selector, 1); client.Submit("execute", selector);
        client.Submit("adjust", selector, 1); client.Submit("adjust", selector, -1); client.Submit("reset", selector);
        var queue = (LinkedList<Intent>)typeof(ControlClient).GetField("queue", BindingFlags.NonPublic | BindingFlags.Instance)!.GetValue(client)!;
        Check(queue.Count == 5 && queue.First!.Value.Ticks == 2, "Coalescing crossed a direction/button/reset barrier");
        for (var i = queue.Count; i < 128; i++) Check(client.Submit("execute", selector), "Queue rejected too early");
        Check(!client.Submit("execute", selector) && queue.Count == 128, "Overflow did not reject explicitly");
    }
    public static void ColorChecks() {
        var type = typeof(PirateWavePlugin).Assembly.GetType("Loupedeck.PirateWavePlugin.ControlFeedback")!;
        var flags = BindingFlags.Static | BindingFlags.NonPublic;
        bool? Active(Entry entry, bool command = true) => (bool?)type.GetMethod("Active", flags)!.Invoke(null, [entry, command]);
        uint Color(bool? active) => (uint)type.GetMethod("Color", flags)!.Invoke(null, [active])!;
        Check(Color(true) == 0xFF4ADE80 && Color(false) == 0xFFF87171 && Color(null) == 0xFFE5E7EB, "Toggle color palette must be green/on, red/off, neutral/unknown");
        var entry = new Entry(new Selector("node", "input", "Mic", "", "toggle_mute", ""), "Toggle mute", "Muted", false, 0, 0, 1, 0, 1, "", []);
        Check(Active(entry) == true && Active(entry with { State = "Unmuted" }) == false, "Legacy mute on/off feedback is incorrect");
        Check(Active(entry with { State = "Enabled" }) == null, "An unrelated rack state must not be interpreted as mute state");
        Check(Active(entry with { Active = false, State = "Localized text" }) == false, "Typed state must be independent of display labels");
        foreach (var pair in new[] { ("node", "toggle_solo", "Solo on", "Solo off"), ("node", "toggle_listening", "Listening", "Not listening"), ("node", "toggle_disabled", "Enabled", "Bypassed"), ("node", "toggle_ducking", "Enabled", "Bypassed"), ("effect", "toggle", "Enabled", "Disabled"), ("application", "toggle_audio", "Running", "Stopped"), ("switch", "activate", "Active", "Inactive"), ("node", "start_recording", "Armed", "Stopped") }) {
            var selected = entry with { Selector = entry.Selector with { Category = pair.Item1, Action = pair.Item2 } };
            Check(Active(selected with { State = pair.Item3 }) == true && Active(selected with { State = pair.Item4 }) == false, "Incorrect live state for " + pair.Item2);
        }
        var volume = entry with { Selector = entry.Selector with { Category = "mixer", Action = "volume" }, Adjustment = true, Active = true };
        Check(Active(volume) == true && Active(volume, false) == null, "Mute press feedback must not recolor ordinary volume rotation");
        using var client = new ControlClient();
        typeof(PirateWavePlugin).GetProperty("Client", BindingFlags.Static | BindingFlags.NonPublic)!.SetValue(null, client);
        typeof(ControlClient).GetField("connected", BindingFlags.NonPublic | BindingFlags.Instance)!.SetValue(client, true);
        typeof(ControlClient).GetField("status", BindingFlags.NonPublic | BindingFlags.Instance)!.SetValue(client, "Connected");
        typeof(ControlClient).GetField("snapshot", BindingFlags.NonPublic | BindingFlags.Instance)!.SetValue(client, new Snapshot(1, "Color", [entry with { Active = true }], ""));
        var action = new NodeCommand(); var binding = BindingOf(action);
        var indicator = binding.GetType().GetMethod("Indicator", BindingFlags.Instance | BindingFlags.NonPublic)!;
        var args = new Loupedeck.ActionEditorActionParameters(new Dictionary<string, string> { ["action"] = JsonSerializer.Serialize(entry.Selector, ControlClient.Json) });
        Check((bool?)indicator.Invoke(binding, [args, true]) == true, "Resolved toggle did not show its live state");
        typeof(ControlClient).GetField("status", BindingFlags.NonPublic | BindingFlags.Instance)!.SetValue(client, "Operation failed");
        Check(indicator.Invoke(binding, [args, true]) == null, "Operation errors must use neutral feedback");
        typeof(ControlClient).GetField("status", BindingFlags.NonPublic | BindingFlags.Instance)!.SetValue(client, "Connected");
        foreach (var entries in new[] { new[] { entry, entry }, Array.Empty<Entry>() }) {
            typeof(ControlClient).GetField("snapshot", BindingFlags.NonPublic | BindingFlags.Instance)!.SetValue(client, new Snapshot(1, "Color", entries, ""));
            Check(indicator.Invoke(binding, [args, true]) == null, "Missing/ambiguous target must not look enabled or disabled");
        }
    }
    static object BindingOf(Loupedeck.ActionEditorAction action) => action.GetType().BaseType!.GetField("binding", BindingFlags.NonPublic | BindingFlags.Instance)!.GetValue(action)!;
    static List<Loupedeck.ActionEditorListboxItem> Items(Loupedeck.ActionEditorAction action, string name, params (string Name, string Value)[] values) {
        var state = new Loupedeck.ActionEditorState(values.Select(v => new Loupedeck.ActionEditorControlState { Name = v.Name, Value = v.Value }).ToArray());
        return ((Loupedeck.ActionEditorListboxItemsRequestedEventArgs)typeof(Loupedeck.ActionEditor).GetMethod("InvokeListboxItemsRequestedEvent", BindingFlags.NonPublic | BindingFlags.Instance)!.Invoke(action.ActionEditor, [state, name])!).Items;
    }
    public static void EditorChecks() {
        using var client = new ControlClient();
        using var plugin = new PirateWavePlugin();
        typeof(PirateWavePlugin).GetProperty("Client", BindingFlags.Static | BindingFlags.NonPublic)!.SetValue(null, client);
        typeof(Loupedeck.Plugin).GetProperty("Name")!.SetValue(plugin, "PirateWave");
        // Initialize through the actual SDK's editor API adapter, then observe
        // the same invalidation event forwarded by the native Logi service.
        var initialize = typeof(Loupedeck.ActionEditorAction).GetMethod("Initialize", BindingFlags.NonPublic | BindingFlags.Instance)!;
        var apiType = initialize.GetParameters()[1].ParameterType;
        var adapterType = typeof(Loupedeck.Plugin).GetNestedTypes(BindingFlags.NonPublic).Single(t => apiType.IsAssignableFrom(t));
        var adapter = Activator.CreateInstance(adapterType, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic, null, [plugin], null)!;
        var action = new NodeCommand();
        initialize.Invoke(action, [plugin, adapter]);
        var invalidations = 0; var feedback = 0;
        var invalidatedControls = new List<string>();
        plugin.ActionEditorListboxItemsChanged += (_, e) => { invalidations++; invalidatedControls.Add(e.ControlName); };
        var binding = BindingOf(action);
        binding.GetType().GetField("Refresh", BindingFlags.NonPublic | BindingFlags.Instance)!.SetValue(binding, (Action)(() => feedback++));
        var update = binding.GetType().GetMethod("Update", BindingFlags.NonPublic | BindingFlags.Instance)!;
        void Publish(bool connected, params Entry[] entries) {
            typeof(ControlClient).GetField("connected", BindingFlags.NonPublic | BindingFlags.Instance)!.SetValue(client, connected);
            // Round-trip to ensure comparisons use content, not array identity.
            var snapshot = JsonSerializer.Deserialize<Snapshot>(JsonSerializer.Serialize(new Snapshot(1, "Editor", entries, ""), ControlClient.Json), ControlClient.Json)!;
            typeof(ControlClient).GetField("snapshot", BindingFlags.NonPublic | BindingFlags.Instance)!.SetValue(client, snapshot);
            update.Invoke(binding, null);
        }
        var selector = new Selector("node", "input", "Mic \"Unicode é\" / |", "", "toggle_mute", "");
        var entry = new Entry(selector, "Toggle mute", "Active", false, 0, 0, 1, 0, 1, "", []);
        Publish(true, entry);
        Check(invalidations == 3, "Initial catalog did not refresh dropdowns");
        var node = Items(action, "Node").Single();
        var target = Items(action, "Target", ("Node", node.Name)).Single();
        var command = Items(action, "Action", ("Node", node.Name), ("Target", target.Name)).Single();
        Check(JsonSerializer.Deserialize<Selector>(command.Name, ControlClient.Json) == selector, "Dependent SDK dropdowns lost their structured selector");
        // The Logi editor normalizes control names to lowercase, even when the
        // SDK declaration uses PascalCase. Exercise that actual request shape.
        foreach (var names in new[] { new[] { "node", "target", "action" }, new[] { "nOdE", "tArGeT", "aCtIoN" } }) {
            var normalizedNode = Items(action, names[0]).Single();
            var normalizedTarget = Items(action, names[1], (names[0], normalizedNode.Name)).Single();
            var normalizedAction = Items(action, names[2], (names[0], normalizedNode.Name), (names[1], normalizedTarget.Name)).Single();
            Check(normalizedAction.Name == command.Name, "Control name casing changed the selected target");
        }
        var lowerParameters = new Loupedeck.ActionEditorActionParameters(new Dictionary<string, string> { ["action"] = command.Name });
        Check((Selector?)binding.GetType().GetMethod("Selected", BindingFlags.Static | BindingFlags.NonPublic)!.Invoke(null, [lowerParameters]) == selector, "Lowercase saved action parameters failed to resolve");
        var initialFeedback = feedback;
        var controlChanged = typeof(Loupedeck.ActionEditor).GetMethod("InvokeControlValueChangedEvent", BindingFlags.NonPublic | BindingFlags.Instance)!;
        foreach (var control in new[] { "Action", "Sensitivity" })
            controlChanged.Invoke(action.ActionEditor, [new Loupedeck.ActionEditorState([]), control]);
        Check(invalidations == 3, "Editing an action/sensitivity reset its own dropdown");
        for (var i = 0; i < 20; i++) Publish(true, entry);
        Check(invalidations == 3 && feedback == initialFeedback, "Identical state packets flooded Logi editor/device events");
        Publish(true, entry with { State = "Muted", Value = 1 });
        Check(invalidations == 3 && feedback == initialFeedback + 1, "Live state must update feedback without resetting dropdowns");
        Publish(true, entry with { Selector = selector with { Node = "Renamed" } });
        Check(invalidations == 6 && Items(action, "Node").Single().DisplayName.Contains("Renamed"), "Renaming did not refresh choices");
        Publish(false);
        Check(invalidations == 9 && Items(action, "Node").Single().Name == "__piratewave_offline__", "Offline editor is silently empty");
        Publish(true);
        Check(Items(action, "Node").Single().Name == "__piratewave_empty__", "Empty category has no explanation");
        foreach (var control in new[] { "node", "nOdE", "target", "tArGeT" }) {
            var before = invalidations;
            var changedState = new Loupedeck.ActionEditorState(new[] { "node", "target", "action" }.Select(name => new Loupedeck.ActionEditorControlState { Name = name, Value = "" }).ToArray());
            controlChanged.Invoke(action.ActionEditor, [changedState, control]);
            Check(invalidations == before + (control.Equals("node", StringComparison.OrdinalIgnoreCase) ? 2 : 1), "Normalized control change failed to refresh dependent choices");
            // The renderer stores messages and list results under exact keys.
            // A response named Target creates a second cache entry instead of
            // updating the visible target control. Do not normalize in this test.
            Check(invalidatedControls.All(name => name is "node" or "target" or "action"), "Refresh event targets a different renderer cache key");
            var messages = (System.Collections.IEnumerable)typeof(Loupedeck.ActionEditorState).GetProperty("Messages", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(changedState)!;
            foreach (var message in messages) {
                var name = (string)message.GetType().GetProperty("ControlName")!.GetValue(message)!;
                Check(name is "target" or "action", "Selection reset targets a different renderer state key");
            }
        }
        Publish(true, entry with { Active = false });
        var beforeActiveFeedback = feedback; var beforeActiveChoices = invalidations;
        Publish(true, entry with { Active = true });
        Check(feedback == beforeActiveFeedback + 1 && invalidations == beforeActiveChoices, "Boolean-state changes must repaint without reloading dropdowns");
    }
    static async Task SlowFeedbackCheck(ControlClient client) {
        using var entered = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        Action slow = () => { entered.Set(); release.Wait(TimeSpan.FromSeconds(8)); };
        client.Changed += slow;
        try {
            await Until(() => entered.IsSet, "Feedback callback was not invoked");
            var before = client.Snapshot;
            // Longer than the transport heartbeat timeout: a blocked SDK callback
            // must not stop catalog reads or cause a disconnect/reconnect cycle.
            await Task.Delay(4500);
            Check(client.Connected && client.Snapshot.Generation != 0 && !ReferenceEquals(before, client.Snapshot), "Slow editor callback blocked controller state reads");
        } finally { client.Changed -= slow; release.Set(); }
    }
    public static void InstallationChecks() {
        // Logi's package verifier only checks metadata/files. Its runtime loader
        // additionally requires exactly one constructible class of each type,
        // including ClientApplication for universal plugins.
        var types = typeof(PirateWavePlugin).Assembly.GetTypes();
        var plugins = types.Where(t => !t.IsAbstract && t.IsSubclassOf(typeof(Loupedeck.Plugin))).ToArray();
        var applications = types.Where(t => !t.IsAbstract && t.IsSubclassOf(typeof(Loupedeck.ClientApplication))).ToArray();
        Check(plugins.Length == 1 && applications.Length == 1, "Logi installation requires one Plugin and one ClientApplication entry point");
        var application = (Loupedeck.ClientApplication)Activator.CreateInstance(applications.Single())!;
        Check(application.GetApplicationStatus() == Loupedeck.ClientApplicationStatus.Unknown, "Universal plugin must not depend on PirateWave being installed or running");
        var plugin = (Loupedeck.Plugin)Activator.CreateInstance(plugins.Single())!;
        try {
            Check(plugin.HasNoApplication && plugin.UsesApplicationApiOnly, "PirateWave must remain a universal API plugin");
            Check(plugin.Install() && plugin.Uninstall(), "SDK installation lifecycle failed while PirateWave was offline");
        } finally { plugin.Unload(); }
    }
    public static async Task Run(string exe) {
        InstallationChecks();
        QueueChecks();
        EditorChecks();
        ColorChecks();
        using var startup = new ControlClient(PipeName); startup.Start();
        await Task.Delay(200); Check(!startup.Connected, "Unexpected controller instance already running");
        using var host = Host(exe);
        try {
            await Until(() => startup.Connected && startup.Snapshot.Generation != 0, "Client-first startup failed");
            startup.Dispose(); await Task.Delay(100); await RawVersionCheck(); await OversizeFrameCheck();
            using var client = new ControlClient(PipeName); client.Start();
            await Until(() => client.Connected && client.Snapshot.Generation != 0, "Client did not connect");
            await SlowFeedbackCheck(client);
            var entry = client.Snapshot.Entries.First(e => e.Selector.Category == "mixer" && e.Selector.NodeKind == "input" && e.Selector.Action == "volume");
            Check(entry.Active == false, "Native catalog did not publish initial mute-off state");
            Check(SdkRun(new MixerAdjustment(), client, entry, "ApplyAdjustment", 5), "SDK adjustment callback rejected");
            await Until(() => client.Find(entry.Selector, out _)?.Value == 5, "Knob value did not update");
            Check(SdkRun(new MixerAdjustment(), client, entry, "RunCommand"), "SDK reset callback rejected");
            await Until(() => client.Find(entry.Selector, out _) is { State: "Muted", Active: true }, "Volume press did not publish mute-on feedback");
            var slot = client.Snapshot.Entries.First(e => e.Selector.Category == "recording_slot" && e.Selector.Action == "capture");
            Check(SdkRun(new RecordingCommand(), client, slot, "RunCommand"), "SDK empty-slot capture rejected");
            await Until(() => client.Snapshot.Notice == "Accepted 3", "Slot action was not accepted");
            var start = client.Snapshot.Entries.First(e => e.Selector.Category == "application" && e.Selector.Action == "start_audio");
            Check(SdkRun(new ApplicationCommand(), client, start, "RunCommand"), "SDK application callback rejected");
            await Until(() => client.Snapshot.Generation == 1_792_746_915_180_750_501UL, "New generation not published");
            Check(client.Submit("adjust", entry.Selector, -2), "Portable name binding rejected");
            await Until(() => client.Find(entry.Selector, out _)?.Value == 3, "Name binding failed after project change");
            var choice = client.Snapshot.Entries.First(e => e.Selector.Category == "effect_parameter" && e.Choices.Length > 1);
            Check(SdkRun(new EffectAdjustment(), client, choice, "ApplyAdjustment", 1), "SDK choice callback rejected");
            await Until(() => client.Find(choice.Selector, out _)?.Value == choice.Value + 1, "Choice knob failed");
            Check(SdkRun(new EffectAdjustment(), client, choice, "RunCommand"), "Choice reset rejected");
            await Until(() => client.Find(choice.Selector, out _)?.Value == choice.Default, "Choice default not restored");
            var vst = client.Snapshot.Entries.First(e => e.Selector.Category == "vst_parameter");
            Check(SdkRun(new VstAdjustment(), client, vst, "ApplyAdjustment", 10), "SDK VST knob rejected");
            await Until(() => Math.Abs((client.Find(vst.Selector, out _)?.Value ?? -1) - 0.6) < 0.00001, "VST normalized knob failed");
            Check(SdkRun(new VstAdjustment(), client, vst, "RunCommand"), "VST reset rejected");
            await Until(() => client.Find(vst.Selector, out _)?.Value == 0.5, "VST default not restored");
            var duck = client.Snapshot.Entries.First(e => e.Selector.Category == "ducking_parameter" && e.Selector.Action == "amount");
            Check(SdkRun(new DuckingAdjustment(), client, duck, "ApplyAdjustment", -3), "SDK duck amount callback rejected");
            await Until(() => client.Find(duck.Selector, out _)?.Value == -21, "Duck amount did not update");
            Check(SdkRun(new DuckingAdjustment(), client, duck, "RunCommand"), "SDK duck reset rejected");
            await Until(() => client.Find(duck.Selector, out _)?.Value == -18, "Duck default not restored");
            var duckMode = client.Snapshot.Entries.First(e => e.Selector.Category == "ducking_parameter" && e.Selector.Action == "detection");
            Check(SdkRun(new DuckingAdjustment(), client, duckMode, "ApplyAdjustment", 1), "SDK duck mode callback rejected");
            await Until(() => client.Find(duckMode.Selector, out _)?.Value == 1, "Speech mode choice failed");
            var duckToggle = client.Snapshot.Entries.First(e => e.Selector.NodeKind == "auto_ducking" && e.Selector.Action == "toggle_ducking");
            Check(SdkRun(new NodeCommand(), client, duckToggle, "RunCommand"), "SDK duck toggle rejected");
            await Until(() => client.Find(duckToggle.Selector, out _)?.State == "Bypassed", "Duck bypass feedback failed");
            if (client.Snapshot.Entries.Any(e => e.Selector.Category == "extension_parameter")) {
                var extension = client.Snapshot.Entries.First(e => e.Selector.Category == "extension_parameter");
                Check(SdkRun(new ExtensionAdjustment(), client, extension, "ApplyAdjustment", -20), "Extension knob callback rejected");
                await Until(() => Math.Abs(client.Find(extension.Selector, out _)!.Value - 0.8) < 0.0001, "Extension knob feedback failed");
                Check(SdkRun(new ExtensionAdjustment(), client, extension, "RunCommand"), "Extension reset callback rejected");
                await Until(() => client.Find(extension.Selector, out _)?.Value == 1, "Extension reset feedback failed");
                var command = client.Snapshot.Entries.First(e => e.Selector.Category == "extension_action" && e.Selector.Target == "");
                Check(SdkRun(new ExtensionCommand(), client, command, "RunCommand"), "Extension action callback rejected");
                await Until(() => client.Find(extension.Selector, out _)?.Value == 0.25, "Extension action feedback failed");
                var width = client.Snapshot.Entries.First(e => e.Selector.Category == "effect_parameter" && e.Selector.Target == "org.piratewave.examples/stereo-width" && e.Selector.Action == "width");
                Check(SdkRun(new EffectAdjustment(), client, width, "ApplyAdjustment", 10), "Extension effect knob rejected");
                await Until(() => Math.Abs(client.Find(width.Selector, out _)!.Value - 1.2) < 0.0001, "Extension effect feedback failed");
            }
            // Construct all SDK action types and validate their dependent selectors.
            foreach (var type in typeof(PirateWavePlugin).Assembly.GetTypes().Where(t => !t.IsAbstract && (t.IsSubclassOf(typeof(Loupedeck.ActionEditorCommand)) || t.IsSubclassOf(typeof(Loupedeck.ActionEditorAdjustment))))) {
                var action = (Loupedeck.ActionEditorAction)Activator.CreateInstance(type)!;
                var controls = (System.Collections.IEnumerable)typeof(Loupedeck.ActionEditor).GetMethod("GetControls", BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public)!.Invoke(action.ActionEditor, null)!;
                Check(controls.Cast<object>().Count() >= 3, "SDK action selectors missing: " + type.Name);
            }
            host.Kill(); await host.WaitForExitAsync();
            await Until(() => !client.Connected, "Client did not detect disconnect");
            Check(!client.Submit("execute", slot.Selector), "Offline action was queued");
            using var restarted = Host(exe);
            try {
                await Until(() => client.Connected && client.Snapshot.Generation == 1_792_746_915_180_750_500UL, "Client did not reconnect");
                await Task.Delay(300);
                Check(client.Snapshot.Notice == "Accepted 0", "Commands replayed after reconnect");
                Check(client.Find(entry.Selector, out _)?.Value == 0, "Stale knob state after reconnect");
            } finally { restarted.Kill(); await restarted.WaitForExitAsync(); }
            Console.WriteLine("PASS: bounded queue/overflow and adjacent-only coalescing; real SDK action construction and simulated button/dial/reset callbacks (including extensions when PIRATEWAVE_EXTENSION_FIXTURE is set); native user-scoped named pipe; both startup orders; incompatible-version/oversized-frame rejection; live mixer/choice/VST/ducking knob and reset state; ducking button and speech mode; dynamic empty-slot action; cross-project names; disconnect/offline rejection/reconnect; no command replay. No physical device inputs synthesized.");
        } finally { if (!host.HasExited) { host.Kill(); await host.WaitForExitAsync(); } }
    }
}
