use crate::{Error, ErrorKind, NodeId, NodeInfo, OutputInfo, Result, Snapshot};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventTopic {
    Outputs,
    Nodes,
    NodeGeometry,
    Config,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Event {
    OutputAdded {
        sequence: u64,
        output: OutputInfo,
    },
    OutputChanged {
        sequence: u64,
        output: OutputInfo,
    },
    OutputRemoved {
        sequence: u64,
        name: String,
    },
    NodeAdded {
        sequence: u64,
        node: NodeInfo,
    },
    NodeChanged {
        sequence: u64,
        node: NodeInfo,
    },
    NodeGeometryChanged {
        sequence: u64,
        id: NodeId,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    NodeRemoved {
        sequence: u64,
        id: NodeId,
    },
    ConfigReloaded {
        sequence: u64,
        accepted: bool,
    },
}

impl Event {
    pub fn sequence(&self) -> u64 {
        match self {
            Self::OutputAdded { sequence, .. }
            | Self::OutputChanged { sequence, .. }
            | Self::OutputRemoved { sequence, .. }
            | Self::NodeAdded { sequence, .. }
            | Self::NodeChanged { sequence, .. }
            | Self::NodeGeometryChanged { sequence, .. }
            | Self::NodeRemoved { sequence, .. }
            | Self::ConfigReloaded { sequence, .. } => *sequence,
        }
    }
}

pub struct Subscription {
    pub initial: Snapshot,
    pub events: EventStream,
}

pub struct EventStream {
    pub(crate) connection: swm_ipc::Connection,
    pub(crate) last_sequence: u64,
}

impl EventStream {
    pub fn next_event(&mut self) -> Result<Event> {
        let envelope = self.connection.receive()?;
        if !envelope.fds.is_empty() {
            return Err(Error::new(
                ErrorKind::Protocol,
                "event carried unexpected file descriptors",
            ));
        }
        let swm_ipc::Response::Event(event) = envelope.response else {
            return Err(Error::new(ErrorKind::Protocol, "expected an event frame"));
        };
        let event = convert_event(event);
        let sequence = event.sequence();
        if sequence != self.last_sequence.saturating_add(1) {
            return Err(Error::new(
                ErrorKind::Protocol,
                format!(
                    "event sequence gap: expected {}, received {sequence}; reconnect for a fresh snapshot",
                    self.last_sequence.saturating_add(1)
                ),
            ));
        }
        self.last_sequence = sequence;
        Ok(event)
    }
}

impl Iterator for EventStream {
    type Item = Result<Event>;
    fn next(&mut self) -> Option<Self::Item> {
        Some(self.next_event())
    }
}

fn convert_event(v: swm_ipc::ApiEvent) -> Event {
    match v {
        swm_ipc::ApiEvent::OutputAdded { sequence, output } => Event::OutputAdded {
            sequence,
            output: output.into(),
        },
        swm_ipc::ApiEvent::OutputChanged { sequence, output } => Event::OutputChanged {
            sequence,
            output: output.into(),
        },
        swm_ipc::ApiEvent::OutputRemoved { sequence, name } => {
            Event::OutputRemoved { sequence, name }
        }
        swm_ipc::ApiEvent::NodeAdded { sequence, node } => Event::NodeAdded {
            sequence,
            node: node.into(),
        },
        swm_ipc::ApiEvent::NodeChanged { sequence, node } => Event::NodeChanged {
            sequence,
            node: node.into(),
        },
        swm_ipc::ApiEvent::NodeGeometryChanged {
            sequence,
            id,
            pos_x,
            pos_y,
            width,
            height,
        } => Event::NodeGeometryChanged {
            sequence,
            id: NodeId::new(id),
            x: pos_x,
            y: pos_y,
            width,
            height,
        },
        swm_ipc::ApiEvent::NodeRemoved { sequence, id } => Event::NodeRemoved {
            sequence,
            id: NodeId::new(id),
        },
        swm_ipc::ApiEvent::ConfigReloaded { sequence, accepted } => {
            Event::ConfigReloaded { sequence, accepted }
        }
    }
}
