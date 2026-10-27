# ABI 1 and instance lifecycle

## Entry point

Export an unmangled C function `const PirateWaveExtensionApi* piratewave_extension_api(void)`. Return a pointer to an immutable table that remains valid while the DLL is loaded. `version` must be 1 and `size` must exactly match the ABI 1 table. See [piratewave_extension.h](piratewave_extension.h). Rust authors use `piratewave_extension_sdk::export_plugin!(MyPlugin)`; never export Rust traits, references, `Vec`, `String`, exceptions or allocator ownership across the DLL boundary.

`describe` returns a borrowed UTF-8 manifest buffer and writes its byte count. Maximum manifest size: 262,144 bytes. It need not be NUL terminated. The host parses it and requires it to match the package manifest after schema defaults are applied. The plugin ID, contribution IDs, parameters and ports cannot change during a session.

## Lifecycle

1. Scanner: load, describe, validate, create each default instance, configure defaults, process a silent block, reset and destroy. The scanner is a separate process with a 10-second aggregate timeout.
2. Startup: load accepted selected versions and retain their libraries in an immutable session catalog.
3. Preparation: create an instance with the descriptor's type index, sample rate, maximum block size, state-schema version and borrowed immutable state bytes. Allocate delay/history/work memory here. Type indices are only resolved handles in this loaded manifest; projects never persist them.
4. Configure: receive validated parameter indices/values. Parameters absent from projects receive descriptor defaults. Handle all values accepted by the descriptor.
5. Process: receive input and output buffer-pointer arrays, counts and frame count. Process all outputs synchronously, preserving input buffers. Return 0 for success or nonzero for failure.
6. Reset: clear histories and settle transient state without allocating, blocking or doing I/O. Bypass and recovery may call reset.
7. Destroy: free instance resources using the plugin's allocator. The host never frees plugin-owned memory. No library can unload while its instance or callback exists.

Create/configure/destroy and graph validation happen on control paths between render calls, never inside device callbacks. Exports use independent file-worker instances. An instance is movable between threads and is exclusively owned: callbacks for the same instance never overlap. Distinct instances can process concurrently, so globals shared across them must not introduce render locks. Libraries stay loaded until session shutdown; updating/enabling/disabling packages requires restart.

## Audio buffers

A frame is two adjacent IEEE 754 `float32` values, left then right. Each input/output pointer addresses `frames * 2` floats. Port arrays follow descriptor order for that instance. IDs are stable identities; array positions are runtime handles only. Buffers are valid only for the call. Inputs and outputs never alias. Zero input count is valid for generators; one or more outputs are required. Host output buffers start silent.

PirateWave supplies 128-frame blocks at 48 kHz. Rack exports pad their final partial block with silence and retain only the requested samples. The host does not insert a transport queue, resample a plugin's ports, or add a processing block. Declare zero latency; other values are rejected. Internal algorithms can still alter phase and sound; zero declared latency is an author contract, not a host proof of every DSP algorithm.

No render callback may allocate/deallocate, lock, log, read/write files, download, create threads, wait on workers, load assets, serialize, panic or call GUI APIs. `set_parameter` must be bounded and cheap; prepare expensive coefficients/resources ahead of time and smooth numerical transitions in process. No host API is callable from processing.

## Errors and state migration

Return null on failed create and nonzero on failed callbacks. The Rust wrapper catches unwind panics; this does not contain aborts, native memory faults or hangs. Faulted processors preserve dry audio, faulted nodes silence all outputs; explicit node bypass uses only declared mappings. Status/logging is published outside rendering, and bounded normal recovery attempts reset instances.

`migrate` is an optional operation implemented by returning failure when unsupported. It receives old state-schema/bytes and a host-owned output buffer of 65,536 bytes; write the migrated bytes and length on success. It runs only during explicit offline project migration. It must not overwrite inputs, retain pointers, or exceed the output capacity. The SDK's default implementation declines migration. No allocation crosses ownership domains: the plugin copies its temporary result into host memory.
