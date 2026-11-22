use std::path::PathBuf;

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(u64);

        impl $name {
            pub const fn new(value: u64) -> Self {
                Self(value)
            }
            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl From<u64> for $name {
            fn from(value: u64) -> Self {
                Self(value)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id_type!(NodeId);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub compositor_version: String,
    pub api_version: u32,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DpmsCommand {
    Off,
    On,
    Toggle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeMoveDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeSelector {
    Focused,
    Latest,
    Id(NodeId),
    Title(String),
    App(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrailDirection {
    Previous,
    Next,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MonitorTarget {
    Direction(Direction),
    Output(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrailTarget {
    Index(usize),
    Selector(NodeSelector),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrailEntry {
    pub index: usize,
    pub cursor: bool,
    pub node: NodeInfo,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrailInfo {
    pub output: String,
    pub cursor_index: Option<usize>,
    pub entries: Vec<TrailEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    Surface,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeState {
    Active,
    Drifting,
    Node,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeRole {
    NormalToplevel,
    Dialog,
    Popup,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeProtocolFamily {
    XdgToplevel,
    XdgPopup,
    Xwayland,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeInfo {
    pub id: NodeId,
    pub title: String,
    pub app_id: Option<String>,
    pub output: Option<String>,
    pub kind: NodeKind,
    pub state: NodeState,
    pub visible: bool,
    pub focused: bool,
    pub latest: bool,
    pub pinned: bool,
    pub role: NodeRole,
    pub protocol_family: NodeProtocolFamily,
    pub modal: bool,
    pub parent: Option<NodeId>,
    pub transient_for: Option<NodeId>,
    pub child_popup_count: usize,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModeInfo {
    pub width: i32,
    pub height: i32,
    pub refresh_millihz: i32,
    pub preferred: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutputInfo {
    pub name: String,
    pub modes: Vec<ModeInfo>,
    pub current_mode: Option<usize>,
    pub offset_x: i32,
    pub offset_y: i32,
    /// Physical pixels per logical desktop unit.
    #[serde(default = "default_output_scale")]
    pub scale: f64,
    pub vrr: String,
    pub vrr_supported: bool,
    pub vrr_active: bool,
}

fn default_output_scale() -> f64 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub sequence: u64,
    pub outputs: Vec<OutputInfo>,
    pub nodes: Vec<NodeInfo>,
    pub config_path: Option<PathBuf>,
}

impl From<swm_ipc::ModeInfo> for ModeInfo {
    fn from(v: swm_ipc::ModeInfo) -> Self {
        Self {
            width: v.width,
            height: v.height,
            refresh_millihz: v.refresh_millihz,
            preferred: v.preferred,
        }
    }
}

impl From<swm_ipc::OutputInfo> for OutputInfo {
    fn from(v: swm_ipc::OutputInfo) -> Self {
        Self {
            name: v.name,
            modes: v.modes.into_iter().map(Into::into).collect(),
            current_mode: v.current_mode,
            offset_x: v.offset_x,
            offset_y: v.offset_y,
            scale: v.scale,
            vrr: v.vrr,
            vrr_supported: v.vrr_supported,
            vrr_active: v.vrr_active,
        }
    }
}

impl From<swm_ipc::NodeInfo> for NodeInfo {
    fn from(v: swm_ipc::NodeInfo) -> Self {
        Self {
            id: NodeId(v.id),
            title: v.title,
            app_id: v.app_id,
            output: v.output,
            kind: match v.kind {
                swm_ipc::NodeKind::Surface => NodeKind::Surface,
            },
            state: match v.state {
                swm_ipc::NodeState::Active => NodeState::Active,
                swm_ipc::NodeState::Drifting => NodeState::Drifting,
                swm_ipc::NodeState::Node => NodeState::Node,
            },
            visible: v.visible,
            focused: v.focused,
            latest: v.latest,
            pinned: v.pinned,
            role: match v.role {
                swm_ipc::NodeRole::NormalToplevel => NodeRole::NormalToplevel,
                swm_ipc::NodeRole::Dialog => NodeRole::Dialog,
                swm_ipc::NodeRole::Popup => NodeRole::Popup,
                swm_ipc::NodeRole::Unknown => NodeRole::Unknown,
            },
            protocol_family: match v.protocol_family {
                swm_ipc::NodeProtocolFamily::XdgToplevel => NodeProtocolFamily::XdgToplevel,
                swm_ipc::NodeProtocolFamily::XdgPopup => NodeProtocolFamily::XdgPopup,
                swm_ipc::NodeProtocolFamily::Xwayland => NodeProtocolFamily::Xwayland,
                swm_ipc::NodeProtocolFamily::Unknown => NodeProtocolFamily::Unknown,
            },
            modal: v.modal,
            parent: v.parent.and_then(|r| r.node_id).map(NodeId),
            transient_for: v.transient_for.and_then(|r| r.node_id).map(NodeId),
            child_popup_count: v.child_popup_count,
            x: v.pos_x,
            y: v.pos_y,
            width: v.width,
            height: v.height,
        }
    }
}

impl From<swm_ipc::TrailListResponse> for TrailInfo {
    fn from(v: swm_ipc::TrailListResponse) -> Self {
        Self {
            output: v.output,
            cursor_index: v.cursor_index,
            entries: v
                .entries
                .into_iter()
                .map(|entry| TrailEntry {
                    index: entry.index,
                    cursor: entry.cursor,
                    node: entry.node.into(),
                })
                .collect(),
        }
    }
}

impl From<swm_ipc::StateSnapshot> for Snapshot {
    fn from(v: swm_ipc::StateSnapshot) -> Self {
        Self {
            sequence: v.sequence,
            outputs: v.outputs.into_iter().map(Into::into).collect(),
            nodes: v.nodes.into_iter().map(Into::into).collect(),
            config_path: v.config_path.map(PathBuf::from),
        }
    }
}
