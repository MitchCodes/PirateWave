# Extension node definitions

A manifest contributes up to 32 types. A session accepts up to 128 total installed contribution types. Each `node` descriptor declares:

| Field | Meaning |
|---|---|
| `id`, `name`, `kind` | Qualified stable identity, display label, `"node"` |
| `description`, `help`, `category` | Concise summary, expanded help, menu grouping |
| `inputs`, `outputs` | Arrays of `{ "id": integer_u32, "name": "Label" }` |
| `parameters`, `actions` | Host-generated controls; see processor guide |
| `state_schema`, `default_state` | Nonzero schema and initial byte array |
| `bypass` | Array of `{ "input": stable_id, "output": stable_id }` |
| `latency_frames` | Must be zero |

Ports have independent identities in each direction. IDs can be sparse; the example uses 101/202 inputs and 303/404/505 outputs. Keep IDs stable across releases. Each direction permits at most eight ports, and outputs cannot be empty. The same source may feed multiple input ports. Multiple sources connected to one port are summed without normalization. Cycles and existing endpoint feedback rules remain prohibited.

PirateWave snapshots the definition into the project. Connections save each exact source node/output and destination node/input. They compile to buffer indices before rendering. Display labels and Routing heights follow the snapshot, including when the DLL is missing. Editing routing, selecting a wire, disconnecting one route and Switch destination selection preserve port identities. Switch selection still activates every route to the selected destination.

Every output is a distinct processed recording tap. A downstream recorder captures the selected output, not an arbitrary first output. Missing inputs start silent. Generators have no inputs and execute once per graph block. They appear with other extension utility nodes; the initial SDK does not expose hardware endpoint callbacks or source-strip capabilities.

Node bypass routes are explicit copies. One input may copy to several outputs; each output can have at most one bypass source. Unmapped outputs are silent. Missing/faulted active nodes silence all outputs rather than guessing how a custom multiport algorithm should pass audio. The descriptor remains available for settings/routing recovery.

Route an ordinary processor before/after an extension node for additional PirateWave effects. ABI 1 node definitions do not add their own PirateWave rack. Node controls provide parameters, actions, keybinds, bypass and expandable help. Controls and routing use stable project instance IDs; the contribution's qualified type ID distinguishes its controller assignments from other extension types.
