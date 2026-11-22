use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use crate::{
    Direction, DpmsCommand, Error, ErrorKind, EventStream, EventTopic, MonitorTarget, NodeInfo,
    NodeMoveDirection, NodeSelector, OutputInfo, Result, SWM_API_VERSION, ServerInfo, Subscription,
    TrailDirection, TrailInfo, TrailTarget,
};

#[derive(Clone, Debug, Default)]
pub struct ConnectOptions {
    pub socket_path: Option<PathBuf>,
    pub read_timeout: Option<Duration>,
    pub write_timeout: Option<Duration>,
}

pub struct Client {
    connection: Mutex<swm_ipc::Connection>,
    socket_path: PathBuf,
    server_info: ServerInfo,
}

impl Client {
    pub fn connect() -> Result<Self> {
        Self::connect_with(ConnectOptions::default())
    }

    pub fn connect_to(path: impl AsRef<Path>) -> Result<Self> {
        Self::connect_with(ConnectOptions {
            socket_path: Some(path.as_ref().to_path_buf()),
            ..Default::default()
        })
    }

    pub fn connect_with(options: ConnectOptions) -> Result<Self> {
        let path = match options.socket_path {
            Some(path) => path,
            None => swm_ipc::default_socket_path()
                .map_err(|e| Error::new(ErrorKind::Connection, e.to_string()))?,
        };
        let mut connection = swm_ipc::Connection::connect_to(&path)?;
        connection
            .set_read_timeout(options.read_timeout)
            .map_err(|e| Error::new(ErrorKind::Connection, e.to_string()))?;
        connection
            .set_write_timeout(options.write_timeout)
            .map_err(|e| Error::new(ErrorKind::Connection, e.to_string()))?;
        let response = request_on(
            &mut connection,
            swm_ipc::Request::Hello(swm_ipc::HelloRequest {
                api_version: SWM_API_VERSION,
            }),
        )?;
        let swm_ipc::Response::Hello(info) = response else {
            return Err(unexpected("hello", response));
        };
        if info.api_version != SWM_API_VERSION {
            return Err(Error::new(
                ErrorKind::VersionMismatch,
                format!(
                    "swm API version mismatch: client {}, compositor {}",
                    SWM_API_VERSION, info.api_version
                ),
            ));
        }
        Ok(Self {
            connection: Mutex::new(connection),
            socket_path: path,
            server_info: ServerInfo {
                compositor_version: info.compositor_version,
                api_version: info.api_version,
                capabilities: info.capabilities,
            },
        })
    }

    pub fn server_info(&self) -> &ServerInfo {
        &self.server_info
    }

    pub fn outputs(&self) -> Result<Vec<OutputInfo>> {
        match self.request(swm_ipc::Request::Outputs)? {
            swm_ipc::Response::Outputs(v) => Ok(v.outputs.into_iter().map(Into::into).collect()),
            other => Err(unexpected("outputs", other)),
        }
    }

    pub fn set_dpms(&self, command: DpmsCommand, output: Option<&str>) -> Result<()> {
        self.ack(swm_ipc::Request::Dpms {
            command: match command {
                DpmsCommand::Off => swm_ipc::DpmsCommand::Off,
                DpmsCommand::On => swm_ipc::DpmsCommand::On,
                DpmsCommand::Toggle => swm_ipc::DpmsCommand::Toggle,
            },
            output: output.map(str::to_owned),
        })
    }

    pub fn nodes(&self, output: Option<&str>) -> Result<Vec<NodeInfo>> {
        match self.request(swm_ipc::Request::Node(swm_ipc::NodeRequest::List {
            output: output.map(str::to_owned),
        }))? {
            swm_ipc::Response::NodeList(v) => Ok(v
                .outputs
                .into_iter()
                .flat_map(|g| g.nodes)
                .map(Into::into)
                .collect()),
            other => Err(unexpected("node list", other)),
        }
    }

    pub fn node_info(
        &self,
        selector: Option<NodeSelector>,
        output: Option<&str>,
    ) -> Result<NodeInfo> {
        self.node_query(swm_ipc::NodeRequest::Info {
            selector: selector.map(selector_wire),
            output: output.map(str::to_owned),
        })
    }
    pub fn focus_node(&self, selector: Option<NodeSelector>, output: Option<&str>) -> Result<()> {
        self.ack(swm_ipc::Request::Node(swm_ipc::NodeRequest::Focus {
            selector: selector.map(selector_wire),
            output: output.map(str::to_owned),
        }))
    }
    pub fn move_node(
        &self,
        direction: NodeMoveDirection,
        selector: Option<NodeSelector>,
        output: Option<&str>,
    ) -> Result<()> {
        self.ack(swm_ipc::Request::Node(swm_ipc::NodeRequest::Move {
            direction: match direction {
                NodeMoveDirection::Left => swm_ipc::NodeMoveDirection::Left,
                NodeMoveDirection::Right => swm_ipc::NodeMoveDirection::Right,
                NodeMoveDirection::Up => swm_ipc::NodeMoveDirection::Up,
                NodeMoveDirection::Down => swm_ipc::NodeMoveDirection::Down,
            },
            selector: selector.map(selector_wire),
            output: output.map(str::to_owned),
        }))
    }
    /// Move by up to `distance` Field units along one axis, stopping at the
    /// first landmark obstruction. Expanded windows may overlap each other.
    /// Requires `node-move-distance-v1`; distance must be in `1..=i32::MAX`.
    pub fn move_node_by(
        &self,
        direction: NodeMoveDirection,
        distance: u32,
        selector: Option<NodeSelector>,
        output: Option<&str>,
    ) -> Result<()> {
        if distance == 0 || distance > i32::MAX as u32 {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "movement distance must be a whole number in 1..=2147483647 Field units",
            ));
        }
        if !self
            .server_info
            .capabilities
            .iter()
            .any(|c| c == "node-move-distance-v1")
        {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "this compositor does not support movement distances; restart swm after upgrading",
            ));
        }
        self.ack(swm_ipc::Request::Node(swm_ipc::NodeRequest::MoveBy {
            direction: match direction {
                NodeMoveDirection::Left => swm_ipc::NodeMoveDirection::Left,
                NodeMoveDirection::Right => swm_ipc::NodeMoveDirection::Right,
                NodeMoveDirection::Up => swm_ipc::NodeMoveDirection::Up,
                NodeMoveDirection::Down => swm_ipc::NodeMoveDirection::Down,
            },
            distance,
            selector: selector.map(selector_wire),
            output: output.map(str::to_owned),
        }))
    }

    /// Place the window's client top-left (or a collapsed marker's center) at
    /// exact coordinates in its monitor's Field. Requires `node-move-to-v1`.
    pub fn move_node_to(
        &self,
        x: i32,
        y: i32,
        selector: Option<NodeSelector>,
        output: Option<&str>,
    ) -> Result<()> {
        if !self
            .server_info
            .capabilities
            .iter()
            .any(|c| c == "node-move-to-v1")
        {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "this compositor does not support exact movement; restart swm after upgrading",
            ));
        }
        self.ack(swm_ipc::Request::Node(swm_ipc::NodeRequest::MoveTo {
            x,
            y,
            selector: selector.map(selector_wire),
            output: output.map(str::to_owned),
        }))
    }

    pub fn close_node(&self, selector: Option<NodeSelector>, output: Option<&str>) -> Result<()> {
        self.ack(swm_ipc::Request::Node(swm_ipc::NodeRequest::Close {
            selector: selector.map(selector_wire),
            output: output.map(str::to_owned),
        }))
    }
    pub fn collapse_node(
        &self,
        selector: Option<NodeSelector>,
        output: Option<&str>,
    ) -> Result<()> {
        self.ack(swm_ipc::Request::Node(swm_ipc::NodeRequest::Collapse {
            selector: selector.map(selector_wire),
            output: output.map(str::to_owned),
        }))
    }
    pub fn restore_node(&self, selector: Option<NodeSelector>, output: Option<&str>) -> Result<()> {
        self.ack(swm_ipc::Request::Node(swm_ipc::NodeRequest::Restore {
            selector: selector.map(selector_wire),
            output: output.map(str::to_owned),
        }))
    }
    pub fn toggle_node(&self, selector: Option<NodeSelector>, output: Option<&str>) -> Result<()> {
        self.ack(swm_ipc::Request::Node(swm_ipc::NodeRequest::Toggle {
            selector: selector.map(selector_wire),
            output: output.map(str::to_owned),
        }))
    }

    pub fn navigate_trail(&self, direction: TrailDirection, output: Option<&str>) -> Result<()> {
        let output = output.map(str::to_owned);
        self.ack(swm_ipc::Request::Trail(match direction {
            TrailDirection::Previous => swm_ipc::TrailRequest::Previous { output },
            TrailDirection::Next => swm_ipc::TrailRequest::Next { output },
        }))
    }

    pub fn trail(&self, output: Option<&str>) -> Result<TrailInfo> {
        match self.request(swm_ipc::Request::Trail(swm_ipc::TrailRequest::List {
            output: output.map(str::to_owned),
        }))? {
            swm_ipc::Response::TrailList(info) => Ok(info.into()),
            other => Err(unexpected("trail list", other)),
        }
    }

    pub fn goto_trail(&self, target: TrailTarget, output: Option<&str>) -> Result<()> {
        let target = match target {
            TrailTarget::Index(index) => swm_ipc::TrailTarget::Index(index),
            TrailTarget::Selector(selector) => {
                swm_ipc::TrailTarget::Selector(selector_wire(selector))
            }
        };
        self.ack(swm_ipc::Request::Trail(swm_ipc::TrailRequest::Goto {
            target,
            output: output.map(str::to_owned),
        }))
    }

    pub fn config_path(&self) -> Result<Option<PathBuf>> {
        match self.request(swm_ipc::Request::ConfigPath)? {
            swm_ipc::Response::ConfigPath(v) => Ok(v.map(PathBuf::from)),
            other => Err(unexpected("config path", other)),
        }
    }
    pub fn reload_config(&self) -> Result<()> {
        self.ack(swm_ipc::Request::ConfigReload)
    }
    pub fn transfer_window(&self, direction: Direction) -> Result<()> {
        self.ack(swm_ipc::Request::Control(
            swm_ipc::ControlRequest::WindowTransfer(direction_wire(direction)),
        ))
    }
    pub fn pan_field(&self, direction: Direction) -> Result<()> {
        self.ack(swm_ipc::Request::Control(
            swm_ipc::ControlRequest::PanField(direction_wire(direction)),
        ))
    }
    pub fn focus_monitor(&self, target: MonitorTarget) -> Result<()> {
        let target = match target {
            MonitorTarget::Direction(direction) => {
                swm_ipc::MonitorFocusTarget::Direction(direction_wire(direction))
            }
            MonitorTarget::Output(output) => swm_ipc::MonitorFocusTarget::Output(output),
        };
        self.ack(swm_ipc::Request::Control(
            swm_ipc::ControlRequest::MonitorFocus(target),
        ))
    }
    pub fn request_quit(&self) -> Result<()> {
        self.ack(swm_ipc::Request::Quit)
    }

    pub fn subscribe(&self, topics: impl IntoIterator<Item = EventTopic>) -> Result<Subscription> {
        let mut connection = swm_ipc::Connection::connect_to(&self.socket_path)?;
        let topics = topics.into_iter().map(topic_wire).collect();
        let response = request_on(
            &mut connection,
            swm_ipc::Request::Subscribe(swm_ipc::SubscribeRequest {
                api_version: SWM_API_VERSION,
                topics,
            }),
        )?;
        let swm_ipc::Response::Subscribed(snapshot) = response else {
            return Err(unexpected("subscription", response));
        };
        let last_sequence = snapshot.sequence;
        Ok(Subscription {
            initial: snapshot.into(),
            events: EventStream {
                connection,
                last_sequence,
            },
        })
    }

    fn node_query(&self, request: swm_ipc::NodeRequest) -> Result<NodeInfo> {
        match self.request(swm_ipc::Request::Node(request))? {
            swm_ipc::Response::NodeInfo(v) => Ok(v.into()),
            other => Err(unexpected("node", other)),
        }
    }
    fn ack(&self, request: swm_ipc::Request) -> Result<()> {
        match self.request(request)? {
            swm_ipc::Response::Ack => Ok(()),
            other => Err(unexpected("acknowledgement", other)),
        }
    }
    fn request(&self, request: swm_ipc::Request) -> Result<swm_ipc::Response> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| Error::new(ErrorKind::Internal, "client connection lock was poisoned"))?;
        request_on(&mut connection, request)
    }
}

fn request_on(
    connection: &mut swm_ipc::Connection,
    request: swm_ipc::Request,
) -> Result<swm_ipc::Response> {
    let envelope = connection.request(&request, &[])?;
    if !envelope.fds.is_empty() {
        return Err(Error::new(
            ErrorKind::Protocol,
            "response carried unexpected file descriptors",
        ));
    }
    match envelope.response {
        swm_ipc::Response::ApiError(error) => Err(error.into()),
        response => Ok(response),
    }
}

fn unexpected(expected: &str, response: swm_ipc::Response) -> Error {
    Error::new(
        ErrorKind::Protocol,
        format!("expected {expected} response, received {response:?}"),
    )
}
fn selector_wire(v: NodeSelector) -> swm_ipc::NodeSelector {
    match v {
        NodeSelector::Focused => swm_ipc::NodeSelector::Focused,
        NodeSelector::Latest => swm_ipc::NodeSelector::Latest,
        NodeSelector::Id(id) => swm_ipc::NodeSelector::Id(id.get()),
        NodeSelector::Title(s) => swm_ipc::NodeSelector::Title(s),
        NodeSelector::App(s) => swm_ipc::NodeSelector::App(s),
    }
}
fn direction_wire(direction: Direction) -> swm_ipc::ControlDirection {
    match direction {
        Direction::Left => swm_ipc::ControlDirection::Left,
        Direction::Right => swm_ipc::ControlDirection::Right,
        Direction::Up => swm_ipc::ControlDirection::Up,
        Direction::Down => swm_ipc::ControlDirection::Down,
    }
}
fn topic_wire(v: EventTopic) -> swm_ipc::EventTopic {
    match v {
        EventTopic::Outputs => swm_ipc::EventTopic::Outputs,
        EventTopic::Nodes => swm_ipc::EventTopic::Nodes,
        EventTopic::NodeGeometry => swm_ipc::EventTopic::NodeGeometry,
        EventTopic::Config => swm_ipc::EventTopic::Config,
    }
}

#[cfg(test)]
mod client_tests {
    use super::*;
    use crate::NodeId;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_SOCKET: AtomicU64 = AtomicU64::new(0);

    fn mock_client(
        capabilities: &[&str],
        run: impl FnOnce(UnixStream) + Send + 'static,
    ) -> (Client, std::thread::JoinHandle<()>) {
        let path = std::env::temp_dir().join(format!(
            "swm-sdk-test-{}-{}.sock",
            std::process::id(),
            NEXT_SOCKET.fetch_add(1, Ordering::Relaxed)
        ));
        let listener = UnixListener::bind(&path).unwrap();
        let capabilities = capabilities.iter().map(|c| c.to_string()).collect();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let bytes = swm_ipc::read_frame(&mut stream).unwrap();
            assert!(matches!(
                swm_ipc::decode_request(&bytes).unwrap(),
                swm_ipc::Request::Hello(_)
            ));
            let response = swm_ipc::Response::Hello(swm_ipc::ServerInfo {
                compositor_version: "test".into(),
                api_version: SWM_API_VERSION,
                ipc_protocol: swm_ipc::SWM_IPC_VERSION,
                capabilities,
            });
            swm_ipc::write_frame(&mut stream, &swm_ipc::encode_response(&response).unwrap())
                .unwrap();
            run(stream);
        });
        let client = Client::connect_to(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        (client, server)
    }

    #[test]
    fn exact_movement_is_capability_gated_and_preserves_signed_coordinates() {
        use std::io::Read;
        let (client, server) = mock_client(&[], |mut stream| {
            assert_eq!(stream.read(&mut [0]).unwrap(), 0);
        });
        assert_eq!(
            client
                .move_node_to(-300, 200, None, None)
                .unwrap_err()
                .kind(),
            ErrorKind::Unsupported
        );
        drop(client);
        server.join().unwrap();
        let (client, server) = mock_client(&["node-move-to-v1"], |mut stream| {
            let request =
                swm_ipc::decode_request(&swm_ipc::read_frame(&mut stream).unwrap()).unwrap();
            assert!(
                matches!(request, swm_ipc::Request::Node(swm_ipc::NodeRequest::MoveTo {
                x: -300, y: 200, selector: Some(swm_ipc::NodeSelector::Id(17)), output: Some(ref output),
            }) if output == "DP-2")
            );
            swm_ipc::write_frame(
                &mut stream,
                &swm_ipc::encode_response(&swm_ipc::Response::Ack).unwrap(),
            )
            .unwrap();
        });
        client
            .move_node_to(
                -300,
                200,
                Some(NodeSelector::Id(NodeId::new(17))),
                Some("DP-2"),
            )
            .unwrap();
        server.join().unwrap();
    }

    #[test]
    fn distance_movement_checks_capability_and_input_before_sending() {
        use std::io::Read;
        let (client, server) = mock_client(&[], |mut stream| {
            assert_eq!(stream.read(&mut [0]).unwrap(), 0);
        });
        assert_eq!(
            client
                .move_node_by(NodeMoveDirection::Right, 10, None, None)
                .unwrap_err()
                .kind(),
            ErrorKind::Unsupported
        );
        for distance in [0, i32::MAX as u32 + 1, u32::MAX] {
            assert_eq!(
                client
                    .move_node_by(NodeMoveDirection::Right, distance, None, None)
                    .unwrap_err()
                    .kind(),
                ErrorKind::InvalidRequest
            );
        }
        drop(client);
        server.join().unwrap();
    }

    #[test]
    fn distance_movement_and_legacy_movement_use_distinct_requests() {
        let (client, server) = mock_client(&["node-move-distance-v1"], |mut stream| {
            for expected in [
                swm_ipc::NodeRequest::MoveBy {
                    direction: swm_ipc::NodeMoveDirection::Up,
                    distance: 10,
                    selector: Some(swm_ipc::NodeSelector::Id(42)),
                    output: Some("DP-1".into()),
                },
                swm_ipc::NodeRequest::Move {
                    direction: swm_ipc::NodeMoveDirection::Left,
                    selector: None,
                    output: None,
                },
            ] {
                let request =
                    swm_ipc::decode_request(&swm_ipc::read_frame(&mut stream).unwrap()).unwrap();
                let swm_ipc::Request::Node(request) = request else {
                    panic!("not a node request")
                };
                assert_eq!(request, expected);
                swm_ipc::write_frame(
                    &mut stream,
                    &swm_ipc::encode_response(&swm_ipc::Response::Ack).unwrap(),
                )
                .unwrap();
            }
        });
        client
            .move_node_by(
                NodeMoveDirection::Up,
                10,
                Some(NodeSelector::Id(NodeId::new(42))),
                Some("DP-1"),
            )
            .unwrap();
        client
            .move_node(NodeMoveDirection::Left, None, None)
            .unwrap();
        drop(client);
        server.join().unwrap();
    }
}
