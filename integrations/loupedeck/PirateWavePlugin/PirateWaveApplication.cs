namespace Loupedeck.PirateWavePlugin;

// The Logi loader requires exactly one ClientApplication subclass even for a
// universal plugin (HasNoApplication). Leave it unlinked: PirateWave uses its pipe
// connection and must remain available regardless of the foreground program.
public sealed class PirateWaveApplication : ClientApplication
{
    protected override string GetProcessName() => "";
    protected override string GetBundleName() => "";
    public override ClientApplicationStatus GetApplicationStatus() => ClientApplicationStatus.Unknown;
}
