# `swm-api`

Supported Rust SDK for external tools talking to swm (`swmctl` and `swm-lift` build on it). Exposes typed requests, semantic IDs/selectors, structured `ErrorKind` handling, and sequenced event streams.

```rust
use swm_api::{Client, EventTopic, NodeSelector};

fn main() -> swm_api::Result<()> {
    let client = Client::connect()?;
    client.focus_node(Some(NodeSelector::Latest), None)?;

    let mut sub = client.subscribe([EventTopic::Nodes])?;
    println!("{} nodes", sub.initial.nodes.len());
    for event in &mut sub.events {
        println!("{:?}", event?);
    }
    Ok(())
}
```

## Capabilities

- **Connection & control**: `Client::connect()`, `connect_to()`, `connect_with()`, `server_info()`, `config_path()`, `reload_config()`, `request_quit()`
- **Outputs**: `outputs()`, `set_dpms()`
- **Nodes**: `nodes()`, `node_info()`, `focus_node()`, `close_node()`, `collapse_node()`, `restore_node()`, `toggle_node()`
- **Movement**: `move_node()` (80 Field units), `move_node_by(direction, distance, selector, output)` (`1..=i32::MAX` Field units; expanded windows overlap each other and stop only at collapsed landmarks, requires `node-move-distance-v1`), `move_node_to(x, y, selector, output)` (exact Field coordinates, requires `node-move-to-v1`). Expanded-window moves use `animations.window-move`.
- **Trail & monitor control**: `trail()`, `navigate_trail()`, `goto_trail()` (`trail-v1`); `transfer_window()`, `pan_field()`, `focus_monitor()` (`control-v1`).

## Cargo features

- `wayland-client` / `wayland-server`: exposes `swm-surface-dismissal-v1` protocol bindings.
- `test-support`: socket test helpers (`test_support::request` / `reply` covering `Hello`, `Subscribe`, `FocusNode`, `Nodes`, `ConfigPath`, and `ReloadConfig`; other operations return `ErrorKind::Unsupported`).

Full contract and protocol notes: [`docs/api.md`](../docs/api.md).
