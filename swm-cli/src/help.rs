pub const HELP: &str = "\
Usage: swmctl <command>

Commands:
  outputs        List connected monitors and their current mode/position
  reload         Reload the selected configuration immediately
  dpms           Control tty output power state
  node           List, inspect, focus, move, collapse, restore, toggle, or close nodes
  trail          Navigate or inspect this monitor's recent-work focus history
  pan            Pan the selected Field: left|right|up|down
  monitor        Focus a monitor or transfer the selected Field window
  portal         Inspect the desktop portal backend
  config         Edit or verify the selected configuration
  quit           Open swm's exit confirmation

Options:
  -h, --help     Print this message
  -V, --version  Print the swmctl version
";

pub const CONFIG_HELP: &str = "\
Usage:
  swmctl config edit
  swmctl config edit -c PATH
  swmctl config edit --config PATH
  swmctl config verify
  swmctl config verify -c PATH
  swmctl config verify --config PATH

`edit` uses $VISUAL, then $EDITOR, and falls back to vi.
";

pub const NODE_HELP: &str = "\
Usage:
  swmctl node list [-o OUTPUT] [--json]
  swmctl node info [SELECTOR] [-o OUTPUT] [--json]
  swmctl node focus [SELECTOR] [-o OUTPUT]
  swmctl node move left|right|up|down [SELECTOR] [-o OUTPUT] [--distance UNITS]
  swmctl node move to X Y [SELECTOR] [-o OUTPUT]
  swmctl node collapse [SELECTOR] [-o OUTPUT]
  swmctl node restore [SELECTOR] [-o OUTPUT]
  swmctl node toggle [SELECTOR] [-o OUTPUT]
  swmctl node close [SELECTOR] [-o OUTPUT]

Selectors:
  focused, latest, ID, id:ID, title:TEXT, app:APP_ID

Movement:
  Without --distance: legacy 80-Field-unit nearest-free placement.
  With --distance: move up to UNITS along the axis, stopping at obstacles.
  UNITS must be a whole number in 1..=2147483647; --distance=UNITS also works.
  Field units are logical coordinates at native zoom, not physical pixels.
  to X Y: exact Field placement; rejects positions blocked by landmarks.
  X/Y are the client top-left, or a collapsed marker's center. Negative values work.
  Expanded windows may overlap each other; pinned objects cannot move.
  Window movement uses animations.window-move; no animation flag is needed.
  Fullscreen, maximized, or currently grabbed windows cannot move.

Markers:
  * focused node
  + latest node
  - other node
";

pub const TRAIL_HELP: &str = "\
Usage:
  swmctl trail prev [-o OUTPUT]
  swmctl trail next [-o OUTPUT]
  swmctl trail list [-o OUTPUT] [--json]
  swmctl trail goto INDEX|SELECTOR [-o OUTPUT]

Trail walks each monitor's recent Field focus history: prev and next move through
it, list prints it, and goto selects an entry directly. It exposes the same
recent work that Alt+Tab cycles as a carousel, with explicit backward/forward
control and scripting.

Selectors:
  focused, latest, ID, id:ID, title:TEXT, app:APP_ID
";

pub const MONITOR_HELP: &str = "\
Usage:
  swmctl monitor focus left|right|up|down|OUTPUT
  swmctl monitor transfer left|right|up|down
";

pub const PORTAL_HELP: &str = "\
Usage:
  swmctl portal status [--json]
  swmctl portal version [--json]
";

pub const DPMS_HELP: &str = "\
Usage:
  swmctl dpms off|on|toggle [-o OUTPUT]
";
