# swm API

`swm-api` is the public Rust SDK (`version 0.1.0`, `SWM_API_VERSION = 1`) for launchers, bars, scripts, and tests. `swm-ipc` (`SWM_IPC_VERSION = 2`) is private postcard wire framing over Unix sockets, not an external compatibility target.

`swm --version` prints swm + API version. `swmctl --version` works offline and prints CLI version only. Use `Client::server_info()` to inspect a live session.

## Setup and client lifecycle

```toml
[dependencies]
swm-api = { path = "../swm/swm-api" }
```

```rust
use swm_api::{Client, NodeSelector};

fn main() -> swm_api::Result<()> {
    let client = Client::connect()?;
    println!("swm {}", client.server_info().compositor_version);
    client.focus_node(Some(NodeSelector::Latest), None)
}
```

- `Client::connect()` connects under `$XDG_RUNTIME_DIR/swm/swm.sock` and runs the API handshake (`connect_to` / `connect_with` support custom socket paths and read/write timeouts).
- `Client` is `Send + Sync`: commands serialize over one persistent socket while `subscribe()` opens a dedicated second socket so slow event consumers never stall requests.

## Operations

| Area | Methods |
| --- | --- |
| Server | `server_info`, `request_quit` |
| Outputs | `outputs`, `set_dpms` |
| Nodes | `nodes`, `node_info`, `focus_node`, `close_node`, `collapse_node`, `restore_node`, `toggle_node` |
| Node motion | `move_node` (80-unit default), `move_node_by` (`node-move-distance-v1`), `move_node_to` (`node-move-to-v1`) |
| Trail (`trail-v1`) | `trail`, `navigate_trail`, `goto_trail` |
| Field & monitor (`control-v1`) | `pan_field`, `focus_monitor`, `transfer_window` |
| Config | `config_path`, `reload_config` |
| Events | `subscribe([EventTopic::{Outputs, Nodes, NodeGeometry, Config}])` |

Selectors (`NodeSelector`) and IDs (`NodeId`) are typed and Serde-serializable. Note: `swm_api::NodeInfo` exposes Field coordinates `x`/`y` (compositor-internal screen rects stay internal to `swm-ipc`).

## Movement contracts

- `move_node_by(direction, distance, selector, output)`: requires `node-move-distance-v1`, accepts `1..=i32::MAX` Field units (`InvalidRequest` otherwise). Sweeps along one axis: expanded windows overlap each other and stop only at collapsed landmarks; collapsed markers stop at both landmarks and non-fullscreen/non-maximized windows. A blocked move is a zero-displacement `Ok(())`; pinned nodes return an error.
- `move_node_to(x, y, selector, output)`: requires `node-move-to-v1`. Places client top-left (or collapsed marker center) at exact signed Field coordinates relative to the node's monitor origin.
- Expanded-window moves use `animations.window-move` (retargeting from visible position); pinned, detached, fullscreen, maximized, grabbed, or animating targets reject movement.
- Private wire stays `SWM_IPC_VERSION = 2` with additive capability strings; rebuild workspace tools alongside compositor updates.

## Subscriptions

`subscribe` delivers an atomic initial snapshot (`Subscription.initial`) plus strictly sequenced deltas (`Subscription.events`). `EventStream` returns `ErrorKind::Protocol` on sequence gaps (reconnect and resync from `initial`). Compositor queues are bounded and drop slow consumers. Subscribe to `EventTopic::NodeGeometry` only when needed.

## Errors

Check `Error::kind()`: `Connection`, `Protocol`, `InvalidRequest`, `NotFound`, `Ambiguous`, `Unsupported`, `VersionMismatch`, `Busy`, `Internal`. Error strings are diagnostic only.

## Wayland surface dismissal (`swm-surface-dismissal-v1`)

Enable `wayland-client` or `wayland-server` on `swm-api` (`swm-api/protocols/swm-surface-dismissal-v1.xml`).

Bind `swm_surface_dismissal_manager_v1` when advertised, call `watch` on a mapped layer-shell surface holding keyboard focus (directly or via popup), and handle one-shot `outside_press` on left-button presses outside the surface tree. swm forwards the press to its real target and never forcibly closes your surface. Unmap/session-lock cancels watches safely; exposes zero foreign coordinates or identities.

## Test helpers (`test-support`)

`swm_api::test_support::{request, reply}` decodes/encodes public SDK types over Unix sockets without depending on `swm-ipc`. Currently covers `Hello`, `Subscribe`, `FocusNode`, `Nodes`, `ConfigPath`, and `ReloadConfig` (other requests return `ErrorKind::Unsupported`).
