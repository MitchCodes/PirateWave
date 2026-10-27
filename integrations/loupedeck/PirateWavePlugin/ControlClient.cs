using System.IO.Pipes;
using System.Security.Principal;
using System.Text.Json;

namespace Loupedeck.PirateWavePlugin;

public sealed record Selector(string Category, string NodeKind, string Node, string Target, string Action, string Plugin);
public sealed record Entry(Selector Selector, string Label, string State, bool Adjustment, double Value, double Min, double Max, double Default, double Step, string Unit, string[] Choices, bool? Active = null);
public sealed record Snapshot(ulong Generation, string Project, Entry[] Entries, string Notice);
public sealed record Intent(string Op, Selector Selector, int Ticks, double Sensitivity, ulong Generation);

// One asynchronous connection; no I/O or waiting in SDK button/dial callbacks.
public sealed class ControlClient : IDisposable
{
    public static readonly JsonSerializerOptions Json = new() { PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower, PropertyNameCaseInsensitive = true };
    private readonly object gate = new();
    private readonly LinkedList<Intent> queue = new();
    private readonly CancellationTokenSource stop = new();
    private Task? worker;
    private int disposed;
    private int notificationPending;
    private int notifying;
    private Snapshot snapshot = new(0, "", [], "");
    private bool connected;
    private string status = "PirateWave offline";
    private readonly string? pipeName;
    // Explicit injection keeps diagnostic clients off the installed plugin's
    // endpoint. Normal SDK construction always uses the current user's pipe.
    public ControlClient(string? pipeName = null) { this.pipeName = pipeName; }
    public event Action? Changed;
    public event Action<string>? Diagnostic;
    private void Trace(string message) { try { Diagnostic?.Invoke(message); } catch { /* Logging must not stop transport. */ } }
    public Snapshot Snapshot { get { lock (gate) return snapshot; } }
    public bool Connected { get { lock (gate) return connected; } }
    public string Status { get { lock (gate) return status; } }
    public void Start() { worker ??= Task.Run(Run); }
    public void Dispose() { if (Interlocked.Exchange(ref disposed, 1) != 0) return; stop.Cancel(); try { worker?.Wait(TimeSpan.FromSeconds(3)); } catch (AggregateException) { } stop.Dispose(); }
    private void Notify()
    {
        // SDK/UI callbacks can block. Keep draining the pipe independently, with
        // one coalesced notification worker rather than an unbounded task queue.
        if (Volatile.Read(ref disposed) != 0) return;
        Interlocked.Exchange(ref notificationPending, 1);
        if (Interlocked.CompareExchange(ref notifying, 1, 0) != 0) return;
        ThreadPool.QueueUserWorkItem(_ => {
            try {
                do {
                    Interlocked.Exchange(ref notificationPending, 0);
                    foreach (var callback in Changed?.GetInvocationList() ?? []) {
                        if (Volatile.Read(ref disposed) != 0) break;
                        try { ((Action)callback)(); } catch (Exception error) { Trace($"PirateWave feedback callback failed: {error.Message}"); }
                    }
                } while (Volatile.Read(ref notificationPending) != 0 && Volatile.Read(ref disposed) == 0);
            } finally {
                Volatile.Write(ref notifying, 0);
                if (Volatile.Read(ref notificationPending) != 0 && Volatile.Read(ref disposed) == 0) Notify();
            }
        });
    }
    public bool Submit(string op, Selector selector, int ticks = 0, double sensitivity = 1)
    {
        lock (gate)
        {
            if (!connected || snapshot.Generation == 0) { status = "PirateWave offline"; return false; }
            if (queue.Count >= 128) { status = "Controller queue full; action rejected"; return false; }
            var intent = new Intent(op, selector, ticks, sensitivity, snapshot.Generation);
            // Only adjacent adjustments may merge; a press/reset is an ordering barrier.
            if (op == "adjust" && queue.Last?.Value is { Op: "adjust" } last && last.Selector == selector && last.Sensitivity == sensitivity && last.Generation == intent.Generation && Math.Sign(last.Ticks) == Math.Sign(ticks) && Math.Abs((long)last.Ticks + ticks) <= 10000)
                queue.Last.Value = last with { Ticks = last.Ticks + ticks };
            else queue.AddLast(intent);
            return true;
        }
    }
    public Entry? Find(Selector selector, out string error)
    {
        lock (gate)
        {
            if (!connected) { error = status; return null; }
            var entries = snapshot.Entries.Where(e => e.Selector == selector).Take(2).ToArray();
            error = entries.Length == 0 ? "Missing target" : entries.Length > 1 ? "Ambiguous target" : "";
            return entries.Length == 1 ? entries[0] : null;
        }
    }
    private async Task Send(Stream pipe, object value, CancellationToken token)
    {
        var bytes = JsonSerializer.SerializeToUtf8Bytes(value, Json);
        await pipe.WriteAsync(BitConverter.GetBytes(bytes.Length), token);
        await pipe.WriteAsync(bytes, token);
        await pipe.FlushAsync(token);
    }
    private async Task<JsonElement> Receive(Stream pipe, CancellationToken token)
    {
        var size = new byte[4]; await pipe.ReadExactlyAsync(size, token);
        var count = BitConverter.ToInt32(size);
        if (count <= 0 || count > 16 * 1024 * 1024) throw new IOException("Invalid controller frame");
        var bytes = new byte[count]; await pipe.ReadExactlyAsync(bytes, token);
        return JsonSerializer.Deserialize<JsonElement>(bytes, Json);
    }
    private void State(JsonElement value)
    {
        var next = value.Deserialize<Snapshot>(Json) ?? throw new IOException("Invalid state");
        bool catalogChanged;
        lock (gate) { catalogChanged = snapshot.Generation != next.Generation || snapshot.Entries.Length != next.Entries.Length; snapshot = next; }
        if (catalogChanged) Trace($"PirateWave catalog: project generation {next.Generation}, {next.Entries.Length} controls");
        Notify();
    }
    private async Task Run()
    {
        while (!stop.IsCancellationRequested)
        {
            try
            {
                var sid = WindowsIdentity.GetCurrent().User?.Value ?? throw new IOException("No Windows user SID");
                using var pipe = new NamedPipeClientStream(".", pipeName ?? $"PirateWave.Control.v1.{sid}", PipeDirection.InOut, PipeOptions.Asynchronous, TokenImpersonationLevel.Identification);
                using var connection = CancellationTokenSource.CreateLinkedTokenSource(stop.Token);
                connection.CancelAfter(TimeSpan.FromSeconds(2));
                await pipe.ConnectAsync(connection.Token);
                ulong id = 1;
                await Send(pipe, new { id, op = "hello", version = 1, client = "loupedeck" }, stop.Token);
                var hello = await Receive(pipe, connection.Token);
                if (!hello.GetProperty("ok").GetBoolean()) throw new IOException("Incompatible PirateWave control protocol");
                await Send(pipe, new { id = ++id, op = "subscribe" }, stop.Token);
                lock (gate) { connected = true; status = "Connected"; queue.Clear(); }
                Trace("Connected to PirateWave; subscribing to current-project controls");
                Notify();
                Task<JsonElement> read = Receive(pipe, stop.Token);
                ulong? awaiting = null;
                var sent = DateTime.UtcNow;
                var heartbeat = DateTime.UtcNow;
                while (!stop.IsCancellationRequested)
                {
                    if (read.IsCompleted)
                    {
                        var response = await read;
                        heartbeat = DateTime.UtcNow;
                        if (response.TryGetProperty("state", out var state)) State(state);
                        if (response.TryGetProperty("id", out var rid) && awaiting == rid.GetUInt64())
                        {
                            lock (gate) status = response.GetProperty("ok").GetBoolean() ? "Accepted" : response.GetProperty("error").GetString() ?? "Action failed";
                            awaiting = null; Notify();
                        }
                        read = Receive(pipe, stop.Token);
                    }
                    if (DateTime.UtcNow - heartbeat > TimeSpan.FromSeconds(4) || (awaiting != null && DateTime.UtcNow - sent > TimeSpan.FromSeconds(3))) throw new IOException("PirateWave connection timed out");
                    if (awaiting == null)
                    {
                        Intent? intent = null;
                        lock (gate) { if (queue.First != null) { intent = queue.First.Value; queue.RemoveFirst(); } }
                        if (intent != null)
                        {
                            if (intent.Generation != Snapshot.Generation) { lock (gate) status = "Project changed; action discarded"; Notify(); }
                            else
                            {
                                awaiting = ++id; sent = DateTime.UtcNow;
                                await Send(pipe, new { id, op = intent.Op, generation = intent.Generation, selector = intent.Selector, ticks = intent.Ticks, sensitivity = intent.Sensitivity }, stop.Token);
                            }
                        }
                        else if (DateTime.UtcNow - sent > TimeSpan.FromSeconds(1))
                        {
                            await Send(pipe, new { id = ++id, op = "state" }, stop.Token); sent = DateTime.UtcNow;
                        }
                    }
                    await Task.Delay(5, stop.Token);
                }
            }
            catch (Exception error)
            {
                var nextStatus = stop.IsCancellationRequested ? "Stopped" : $"PirateWave offline: {error.Message}";
                bool changed;
                lock (gate) { changed = status != nextStatus; status = nextStatus; }
                if (changed) Trace(nextStatus);
            }
            finally { lock (gate) { connected = false; queue.Clear(); snapshot = new(0, "", [], ""); } Notify(); }
            try { await Task.Delay(500, stop.Token); } catch (OperationCanceledException) { break; }
        }
    }
}
