use std::cell::RefCell;
use std::collections::HashSet;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use calloop::channel::{Event, Sender, channel};
use calloop::generic::Generic;
use calloop::{Interest, LoopHandle, Mode as CalloopMode, PostAction};
use smithay::output::Mode as OutputMode;

use smithay::wayland::seat::WaylandFocus;
const MAX_REQUEST_FDS: usize = 32;
const MAX_CLIENTS: usize = 64;
const FRAME_TIMEOUT: Duration = Duration::from_secs(30);
const REPLY_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Default)]
struct ConnectionLimit(Arc<AtomicUsize>);
struct ConnectionSlot(Arc<AtomicUsize>);
impl ConnectionLimit {
    fn acquire(&self) -> Option<ConnectionSlot> {
        self.0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_CLIENTS).then_some(count + 1)
            })
            .ok()
            .map(|_| ConnectionSlot(self.0.clone()))
    }
}
impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

enum ClientEvent {
    Request(RequestEnvelope),
    Closed(u64),
}
struct ClientLifetime {
    id: u64,
    events: Sender<ClientEvent>,
    _slot: ConnectionSlot,
}
impl Drop for ClientLifetime {
    fn drop(&mut self) {
        let _ = self.events.send(ClientEvent::Closed(self.id));
    }
}

fn peer_closed(stream: &UnixStream) -> bool {
    let mut fd = libc::pollfd {
        fd: stream.as_raw_fd(),
        events: libc::POLLRDHUP,
        revents: 0,
    };
    (unsafe { libc::poll(&mut fd, 1, 0) }) > 0
        && fd.revents & (libc::POLLHUP | libc::POLLRDHUP | libc::POLLERR | libc::POLLNVAL) != 0
}

fn wait_reply<T>(
    receiver: &mpsc::Receiver<T>,
    stream: &UnixStream,
    deadline: Option<Instant>,
) -> Option<T> {
    loop {
        if peer_closed(stream) || deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return None;
        }
        match receiver.recv_timeout(Duration::from_secs(1)) {
            Ok(reply) => return Some(reply),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// Backend-agnostic access to per-output metadata for IPC.
pub trait OutputInfoSource {
    fn output_info(&self) -> Vec<swm_ipc::OutputInfo>;
}

/// Shared by both backends' `OutputInfoSource` impls - keeps Smithay's
/// millihertz refresh representation intact on the wire.
pub fn mode_info(mode: OutputMode, preferred: bool) -> swm_ipc::ModeInfo {
    swm_ipc::ModeInfo {
        width: mode.size.w,
        height: mode.size.h,
        refresh_millihz: mode.refresh,
        preferred,
    }
}

pub fn vrr_str(vrr: swm_config::Vrr) -> &'static str {
    match vrr {
        swm_config::Vrr::Off => "off",
        swm_config::Vrr::On => "on",
        swm_config::Vrr::Auto => "auto",
    }
}

fn version_info() -> swm_ipc::VersionInfo {
    swm_ipc::VersionInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        ipc_protocol: swm_ipc::SWM_IPC_VERSION,
    }
}

struct ReplyFrame {
    response: swm_ipc::Response,
    fds: Vec<OwnedFd>,
    stream: Option<mpsc::Receiver<swm_ipc::Response>>,
}

/// The reply half of one IPC request. It is deliberately consumed by
/// `send`, making double replies impossible while allowing a user-driven
/// operation to retain it until selection completes.
pub struct ReplySender(mpsc::SyncSender<ReplyFrame>);

impl ReplySender {
    pub fn send(
        self,
        response: swm_ipc::Response,
        fds: Vec<OwnedFd>,
    ) -> Result<(), Box<swm_ipc::Response>> {
        self.0
            .send(ReplyFrame {
                response,
                fds,
                stream: None,
            })
            .map_err(|err| Box::new(err.0.response))
    }

    pub fn subscribe(
        self,
        snapshot: swm_ipc::StateSnapshot,
        stream: mpsc::Receiver<swm_ipc::Response>,
    ) -> Result<(), Box<swm_ipc::Response>> {
        self.0
            .send(ReplyFrame {
                response: swm_ipc::Response::Subscribed(snapshot),
                fds: Vec::new(),
                stream: Some(stream),
            })
            .map_err(|err| Box::new(err.0.response))
    }
}

/// One request delivered on the compositor thread.
pub struct RequestEnvelope {
    pub client_id: u64,
    pub request: swm_ipc::Request,
    pub fds: Vec<OwnedFd>,
    pub reply: ReplySender,
}

fn client_worker(stream: UnixStream, lifetime: ClientLifetime) {
    if stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .is_err()
    {
        return;
    }
    let requests = &lifetime.events;
    loop {
        let (bytes, fds) =
            match swm_ipc::read_frame_with_fds_timeout(&stream, MAX_REQUEST_FDS, FRAME_TIMEOUT) {
                Ok(frame) => frame,
                Err(swm_ipc::CodecError::Io(err))
                    if matches!(
                        err.kind(),
                        std::io::ErrorKind::UnexpectedEof
                            | std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::BrokenPipe
                    ) =>
                {
                    break;
                }
                Err(err) => {
                    eventline::warn!("ipc: failed to read request: {err}");
                    break;
                }
            };
        let request = match swm_ipc::decode_request(&bytes) {
            Ok(request) => request,
            Err(err) => {
                if write_response(
                    &stream,
                    ReplyFrame {
                        response: api_error(
                            swm_ipc::ServerErrorKind::InvalidRequest,
                            err.to_string(),
                        ),
                        fds: Vec::new(),
                        stream: None,
                    },
                )
                .is_err()
                {
                    break;
                }
                continue;
            }
        };

        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        if requests
            .send(ClientEvent::Request(RequestEnvelope {
                client_id: lifetime.id,
                request,
                fds,
                reply: ReplySender(reply_tx),
            }))
            .is_err()
        {
            break;
        }
        let Some(reply) = wait_reply(&reply_rx, &stream, Some(Instant::now() + REPLY_TIMEOUT))
        else {
            break;
        };
        let mut reply = reply;
        let event_stream = reply.stream.take();
        if let Err(err) = write_response(&stream, reply) {
            eventline::warn!("ipc: failed to write response: {err}");
            break;
        }
        if let Some(event_stream) = event_stream {
            while let Some(response) = wait_reply(&event_stream, &stream, None) {
                if write_response(
                    &stream,
                    ReplyFrame {
                        response,
                        fds: Vec::new(),
                        stream: None,
                    },
                )
                .is_err()
                {
                    break;
                }
            }
            break;
        }
    }
}

fn write_response(stream: &UnixStream, reply: ReplyFrame) -> Result<(), swm_ipc::CodecError> {
    let bytes = swm_ipc::encode_response(&reply.response)?;
    let fds = reply.fds.iter().map(AsRawFd::as_raw_fd).collect::<Vec<_>>();
    swm_ipc::write_frame_with_fds(stream, &bytes, &fds)
}

struct ApiSubscriber {
    client_id: u64,
    topics: HashSet<swm_ipc::EventTopic>,
    sender: mpsc::SyncSender<swm_ipc::Response>,
    sequence: u64,
}

#[derive(Default)]
pub struct ApiSubscriptions {
    previous: Option<swm_ipc::StateSnapshot>,
    subscribers: Vec<ApiSubscriber>,
}

impl ApiSubscriptions {
    pub fn disconnect(&mut self, client_id: u64) {
        self.subscribers
            .retain(|subscriber| subscriber.client_id != client_id);
    }
    fn subscribe(
        &mut self,
        client_id: u64,
        topics: Vec<swm_ipc::EventTopic>,
        snapshot: swm_ipc::StateSnapshot,
    ) -> mpsc::Receiver<swm_ipc::Response> {
        self.previous = Some(snapshot);
        let (sender, receiver) = mpsc::sync_channel(256);
        self.subscribers.push(ApiSubscriber {
            client_id,
            topics: topics.into_iter().collect(),
            sender,
            sequence: 0,
        });
        receiver
    }

    fn publish(&mut self, snapshot: swm_ipc::StateSnapshot) {
        let Some(previous) = self.previous.replace(snapshot.clone()) else {
            return;
        };
        let changes = api_changes(&previous, &snapshot);
        self.subscribers.retain_mut(|subscriber| {
            for (topic, event) in &changes {
                if !subscriber.topics.contains(topic) {
                    continue;
                }
                subscriber.sequence = subscriber.sequence.saturating_add(1);
                let response = swm_ipc::Response::Event(event_with_sequence(
                    event.clone(),
                    subscriber.sequence,
                ));
                if subscriber.sender.try_send(response).is_err() {
                    return false;
                }
            }
            true
        });
    }

    fn publish_event(&mut self, topic: swm_ipc::EventTopic, event: swm_ipc::ApiEvent) {
        self.subscribers.retain_mut(|subscriber| {
            if !subscriber.topics.contains(&topic) {
                return true;
            }
            subscriber.sequence = subscriber.sequence.saturating_add(1);
            subscriber
                .sender
                .try_send(swm_ipc::Response::Event(event_with_sequence(
                    event.clone(),
                    subscriber.sequence,
                )))
                .is_ok()
        });
    }
}

fn event_with_sequence(event: swm_ipc::ApiEvent, sequence: u64) -> swm_ipc::ApiEvent {
    use swm_ipc::ApiEvent;
    match event {
        ApiEvent::OutputAdded { output, .. } => ApiEvent::OutputAdded { sequence, output },
        ApiEvent::OutputChanged { output, .. } => ApiEvent::OutputChanged { sequence, output },
        ApiEvent::OutputRemoved { name, .. } => ApiEvent::OutputRemoved { sequence, name },
        ApiEvent::NodeAdded { node, .. } => ApiEvent::NodeAdded { sequence, node },
        ApiEvent::NodeChanged { node, .. } => ApiEvent::NodeChanged { sequence, node },
        ApiEvent::NodeGeometryChanged {
            id,
            pos_x,
            pos_y,
            width,
            height,
            ..
        } => ApiEvent::NodeGeometryChanged {
            sequence,
            id,
            pos_x,
            pos_y,
            width,
            height,
        },
        ApiEvent::NodeRemoved { id, .. } => ApiEvent::NodeRemoved { sequence, id },
        ApiEvent::ConfigReloaded { accepted, .. } => {
            ApiEvent::ConfigReloaded { sequence, accepted }
        }
    }
}

fn api_changes(
    before: &swm_ipc::StateSnapshot,
    after: &swm_ipc::StateSnapshot,
) -> Vec<(swm_ipc::EventTopic, swm_ipc::ApiEvent)> {
    use swm_ipc::{ApiEvent as E, EventTopic as T};
    let mut changes = Vec::new();
    diff_by_key(
        &before.outputs,
        &after.outputs,
        |v| v.name.clone(),
        DiffEventMappers {
            added: |output| E::OutputAdded {
                sequence: 0,
                output,
            },
            changed: |output| E::OutputChanged {
                sequence: 0,
                output,
            },
            removed: |name| E::OutputRemoved { sequence: 0, name },
            topic: T::Outputs,
        },
        &mut changes,
    );
    let before_nodes = before
        .nodes
        .iter()
        .map(|v| (v.id, v))
        .collect::<std::collections::HashMap<_, _>>();
    let after_nodes = after
        .nodes
        .iter()
        .map(|v| (v.id, v))
        .collect::<std::collections::HashMap<_, _>>();
    for (&id, node) in &after_nodes {
        match before_nodes.get(&id) {
            None => changes.push((
                T::Nodes,
                E::NodeAdded {
                    sequence: 0,
                    node: (*node).clone(),
                },
            )),
            Some(old) => {
                if node_semantics_changed(old, node) {
                    changes.push((
                        T::Nodes,
                        E::NodeChanged {
                            sequence: 0,
                            node: (*node).clone(),
                        },
                    ));
                }
                if old.pos_x != node.pos_x
                    || old.pos_y != node.pos_y
                    || old.width != node.width
                    || old.height != node.height
                {
                    changes.push((
                        T::NodeGeometry,
                        E::NodeGeometryChanged {
                            sequence: 0,
                            id,
                            pos_x: node.pos_x,
                            pos_y: node.pos_y,
                            width: node.width,
                            height: node.height,
                        },
                    ));
                }
            }
        }
    }
    for &id in before_nodes.keys() {
        if !after_nodes.contains_key(&id) {
            changes.push((T::Nodes, E::NodeRemoved { sequence: 0, id }));
        }
    }
    changes
}

struct DiffEventMappers<FA, FC, FR> {
    added: FA,
    changed: FC,
    removed: FR,
    topic: swm_ipc::EventTopic,
}

fn diff_by_key<T, K, FA, FC, FR>(
    before: &[T],
    after: &[T],
    key: impl Fn(&T) -> K,
    events: DiffEventMappers<FA, FC, FR>,
    output: &mut Vec<(swm_ipc::EventTopic, swm_ipc::ApiEvent)>,
) where
    T: Clone + PartialEq,
    K: Clone + Eq + std::hash::Hash,
    FA: Fn(T) -> swm_ipc::ApiEvent,
    FC: Fn(T) -> swm_ipc::ApiEvent,
    FR: Fn(K) -> swm_ipc::ApiEvent,
{
    let DiffEventMappers {
        added,
        changed,
        removed,
        topic,
    } = events;
    let before = before
        .iter()
        .map(|v| (key(v), v))
        .collect::<std::collections::HashMap<_, _>>();
    let after = after
        .iter()
        .map(|v| (key(v), v))
        .collect::<std::collections::HashMap<_, _>>();
    for (id, value) in &after {
        match before.get(id) {
            None => output.push((topic, added((*value).clone()))),
            Some(old) if *old != *value => output.push((topic, changed((*value).clone()))),
            _ => {}
        }
    }
    for id in before.keys() {
        if !after.contains_key(id) {
            output.push((topic, removed(id.clone())));
        }
    }
}

fn node_semantics_changed(a: &swm_ipc::NodeInfo, b: &swm_ipc::NodeInfo) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    (a.pos_x, a.pos_y, a.width, a.height) = (0.0, 0.0, 0.0, 0.0);
    (b.pos_x, b.pos_y, b.width, b.height) = (0.0, 0.0, 0.0, 0.0);
    a.screen = None;
    b.screen = None;
    a != b
}

/// Rejects startup if another listener holds `path`; removes a stale socket first.
fn remove_stale_socket(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    match UnixStream::connect(path) {
        Ok(_) => Err(std::io::Error::other(
            "another swm IPC listener is already active on this socket",
        )),
        Err(_) => std::fs::remove_file(path),
    }
}

/// Binds the IPC socket and routes decoded requests onto the compositor
/// loop. Socket I/O waits on per-connection workers, so a deferred portal
/// reply never blocks rendering or input dispatch.
pub fn init_ipc_listener<App: 'static>(
    loop_handle: &LoopHandle<'_, App>,
    handler: impl Fn(&mut App, RequestEnvelope) + 'static,
    disconnected: impl Fn(&mut App, u64) + 'static,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = swm_ipc::ensure_runtime_dir()?.join("swm.sock");
    remove_stale_socket(&path)?;

    let listener = UnixListener::bind(&path)?;
    listener.set_nonblocking(true)?;

    let (request_tx, request_rx) = channel();
    loop_handle.insert_source(request_rx, move |event, _, app| match event {
        Event::Msg(ClientEvent::Request(request)) => handler(app, request),
        Event::Msg(ClientEvent::Closed(id)) => disconnected(app, id),
        Event::Closed => {}
    })?;

    let limit = ConnectionLimit::default();
    loop_handle.insert_source(
        Generic::new(listener, Interest::READ, CalloopMode::Level),
        move |_, listener, _app| {
            // Yield to rendering even during a connection flood.
            for _ in 0..32 {
                match listener.accept() {
                    Ok((stream, _addr)) => {
                        let Some(slot) = limit.acquire() else {
                            continue;
                        };
                        static NEXT_CLIENT: AtomicU64 = AtomicU64::new(1);
                        let lifetime = ClientLifetime {
                            id: NEXT_CLIENT.fetch_add(1, Ordering::Relaxed),
                            events: request_tx.clone(),
                            _slot: slot,
                        };
                        if let Err(err) = std::thread::Builder::new()
                            .name("swm-ipc-client".to_string())
                            .spawn(move || client_worker(stream, lifetime))
                        {
                            eventline::error!("ipc: failed to start client worker: {err}");
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(err) => {
                        eventline::error!("ipc: accept failed: {err}");
                        break;
                    }
                }
            }
            Ok(PostAction::Continue)
        },
    )?;

    Ok(())
}

pub fn handle_request<D: crate::session::SessionDriver>(
    app: &mut crate::session::Session<D>,
    request: RequestEnvelope,
) {
    let RequestEnvelope {
        client_id,
        request,
        fds,
        reply,
    } = request;
    let accepts_descriptors = matches!(
        request,
        swm_ipc::Request::RegisterDmabuf(_) | swm_ipc::Request::CaptureFrame(_)
    );
    if !accepts_descriptors && !fds.is_empty() {
        let _ = reply.send(
            api_error(
                swm_ipc::ServerErrorKind::InvalidRequest,
                "request included unexpected descriptors",
            ),
            Vec::new(),
        );
        return;
    }
    if let swm_ipc::Request::Subscribe(subscription) = &request {
        if subscription.api_version != swm_ipc::SWM_API_VERSION {
            let _ = reply.send(
                swm_ipc::Response::ApiError(swm_ipc::ServerError::new(
                    swm_ipc::ServerErrorKind::VersionMismatch,
                    format!("unsupported API version {}", subscription.api_version),
                )),
                Vec::new(),
            );
            return;
        }
        publish_api_events(app);
        let snapshot = api_snapshot(app);
        let stream = app.api_subscriptions.subscribe(
            client_id,
            subscription.topics.clone(),
            snapshot.clone(),
        );
        let _ = reply.subscribe(snapshot, stream);
        return;
    }
    let response = match request {
        swm_ipc::Request::Outputs => swm_ipc::Response::Outputs(swm_ipc::OutputsResponse {
            outputs: app.driver.output_info(),
        }),
        swm_ipc::Request::Version => swm_ipc::Response::Version(version_info()),
        swm_ipc::Request::ChooseSource(request) => {
            crate::capture::request_source(app, request, reply);
            return;
        }
        swm_ipc::Request::CancelSourceChooser { request_handle } => {
            if crate::capture::cancel_portal(app, &request_handle) {
                swm_ipc::Response::Ack
            } else {
                api_error(
                    swm_ipc::ServerErrorKind::NotFound,
                    "source chooser is not active",
                )
            }
        }
        swm_ipc::Request::BeginWindowCast { window_id } => {
            match crate::capture::resolve_source_window(&app.wayland.space, &app.nodes, window_id) {
                Ok(window) => {
                    if let Some(surface) = window.wl_surface().map(|surface| surface.into_owned()) {
                        if app.window_rules.applied(&surface).block_screencast {
                            api_error(
                                swm_ipc::ServerErrorKind::InvalidRequest,
                                "window is protected from screencasting",
                            )
                        } else {
                            app.capture.screencast.begin_window_cast(client_id, surface);
                            app.refresh_window_cast_rules();
                            app.request_redraw();
                            swm_ipc::Response::Ack
                        }
                    } else {
                        api_error(
                            swm_ipc::ServerErrorKind::NotFound,
                            "window is no longer available",
                        )
                    }
                }
                Err(err) => api_error(swm_ipc::ServerErrorKind::NotFound, err.to_string()),
            }
        }
        swm_ipc::Request::RegisterDmabuf(request) => {
            match app.capture.screencast.register(client_id, request, fds) {
                Ok(()) => swm_ipc::Response::Ack,
                Err(error) => swm_ipc::Response::ApiError(error),
            }
        }
        swm_ipc::Request::RemoveDmabuf {
            stream_handle,
            buffer_id,
        } => {
            app.capture
                .screencast
                .remove(client_id, &stream_handle, buffer_id);
            swm_ipc::Response::Ack
        }
        swm_ipc::Request::CaptureFrame(request) => {
            match crate::capture::screencast::capture_frame(app, client_id, request, fds) {
                Ok(crate::capture::screencast::CaptureFrameResult::Immediate(response)) => {
                    swm_ipc::Response::Frame(response)
                }
                Ok(crate::capture::screencast::CaptureFrameResult::Submitted {
                    response,
                    sync,
                }) => {
                    let pending_reply = Rc::new(RefCell::new(Some(reply)));
                    let completion_reply = pending_reply.clone();
                    let completion = Box::new(move || {
                        if let Some(reply) = completion_reply.borrow_mut().take() {
                            let _ = reply.send(swm_ipc::Response::Frame(response), Vec::new());
                        }
                    });
                    if let Err(message) = app.driver.schedule_render_completion(sync, completion)
                        && let Some(reply) = pending_reply.borrow_mut().take()
                    {
                        let _ = reply.send(
                            api_error(swm_ipc::ServerErrorKind::Internal, message),
                            Vec::new(),
                        );
                    }
                    return;
                }
                Err(error) => swm_ipc::Response::ApiError(error),
            }
        }
        swm_ipc::Request::Node(request) => crate::nodes::handle_request(app, request),
        swm_ipc::Request::Quit => {
            app.show_exit_confirmation();
            swm_ipc::Response::Ack
        }
        swm_ipc::Request::ConfigPath => swm_ipc::Response::ConfigPath(
            app.config_source
                .path()
                .map(|path| path.to_string_lossy().into_owned()),
        ),
        swm_ipc::Request::Dpms { command, output } => {
            app.request_dpms(command, output, move |app, result| {
                let response = match result {
                    Ok(()) => {
                        crate::wayland::session_lock::confirm_unlit_outputs(app);
                        swm_ipc::Response::Ack
                    }
                    Err(error) => swm_ipc::Response::ApiError(error),
                };
                let _ = reply.send(response, Vec::new());
            });
            return;
        }
        swm_ipc::Request::CaptureCapabilities => {
            let capabilities = app.driver.dmabuf_capabilities();
            swm_ipc::Response::CaptureCapabilities(swm_ipc::CaptureCapabilities {
                main_device: capabilities.main_device(),
                dmabuf_formats: capabilities
                    .formats()
                    .iter()
                    .map(|format| swm_ipc::DmabufFormat {
                        fourcc: format.code as u32,
                        modifier: format.modifier.into(),
                    })
                    .collect(),
            })
        }
        swm_ipc::Request::Hello(hello) => {
            if hello.api_version != swm_ipc::SWM_API_VERSION {
                swm_ipc::Response::ApiError(swm_ipc::ServerError::new(
                    swm_ipc::ServerErrorKind::VersionMismatch,
                    format!("unsupported API version {}", hello.api_version),
                ))
            } else {
                swm_ipc::Response::Hello(swm_ipc::ServerInfo {
                    compositor_version: env!("CARGO_PKG_VERSION").to_string(),
                    api_version: swm_ipc::SWM_API_VERSION,
                    ipc_protocol: swm_ipc::SWM_IPC_VERSION,
                    capabilities: vec![
                        "commands-v1".into(),
                        "subscriptions-v1".into(),
                        "trail-v1".into(),
                        "control-v1".into(),
                        "node-move-distance-v1".into(),
                        "node-move-to-v1".into(),
                    ],
                })
            }
        }
        swm_ipc::Request::Subscribe(_) => unreachable!(),
        swm_ipc::Request::ConfigReload => match app.config_source.request_reload() {
            true => swm_ipc::Response::Ack,
            false => swm_ipc::Response::ApiError(swm_ipc::ServerError::new(
                swm_ipc::ServerErrorKind::NotFound,
                "no configuration file is being watched",
            )),
        },
        swm_ipc::Request::Trail(request) => crate::trail::handle_request(app, request),
        swm_ipc::Request::Control(request) => handle_control_request(app, request),
    };
    let _ = reply.send(response, Vec::new());
}

fn handle_control_request<D: crate::session::SessionDriver>(
    session: &mut crate::session::Session<D>,
    request: swm_ipc::ControlRequest,
) -> swm_ipc::Response {
    let direction = |direction| match direction {
        swm_ipc::ControlDirection::Left => swm_config::Direction::Left,
        swm_ipc::ControlDirection::Right => swm_config::Direction::Right,
        swm_ipc::ControlDirection::Up => swm_config::Direction::Up,
        swm_ipc::ControlDirection::Down => swm_config::Direction::Down,
    };
    let action = match request {
        swm_ipc::ControlRequest::WindowTransfer(value) => {
            return match crate::nodes::transfer_window(session, direction(value)) {
                Ok(()) => swm_ipc::Response::Ack,
                Err(message) => api_error(swm_ipc::ServerErrorKind::InvalidRequest, message),
            };
        }
        swm_ipc::ControlRequest::PanField(value) => {
            return match crate::nodes::pan_field(session, direction(value)) {
                Ok(()) => swm_ipc::Response::Ack,
                Err(message) => api_error(swm_ipc::ServerErrorKind::InvalidRequest, message),
            };
        }
        swm_ipc::ControlRequest::MonitorFocus(target) => {
            let target = match target {
                swm_ipc::MonitorFocusTarget::Direction(value) => {
                    swm_config::MonitorTarget::Direction(direction(value))
                }
                swm_ipc::MonitorFocusTarget::Output(name) => {
                    if !session
                        .wayland
                        .space
                        .outputs()
                        .any(|output| output.name() == name)
                    {
                        return api_error(
                            swm_ipc::ServerErrorKind::NotFound,
                            format!("unknown output {name:?}"),
                        );
                    }
                    swm_config::MonitorTarget::Output(name)
                }
            };
            swm_config::Action::MonitorFocus(target)
        }
    };
    let Some(socket_name) = session.wayland_display.clone() else {
        return api_error(
            swm_ipc::ServerErrorKind::Internal,
            "Wayland display is not ready",
        );
    };
    crate::session::input::actions::dispatch(
        session,
        action,
        &socket_name,
        None,
        crate::session::input::actions::DispatchOrigin::Other,
    );
    swm_ipc::Response::Ack
}

fn api_error(kind: swm_ipc::ServerErrorKind, message: impl Into<String>) -> swm_ipc::Response {
    swm_ipc::Response::ApiError(swm_ipc::ServerError::new(kind, message))
}

fn api_snapshot<D: crate::session::SessionDriver>(
    app: &mut crate::session::Session<D>,
) -> swm_ipc::StateSnapshot {
    let nodes = match crate::nodes::handle_request(app, swm_ipc::NodeRequest::List { output: None })
    {
        swm_ipc::Response::NodeList(list) => list
            .outputs
            .into_iter()
            .flat_map(|group| group.nodes)
            .collect(),
        _ => Vec::new(),
    };
    swm_ipc::StateSnapshot {
        sequence: 0,
        outputs: app.driver.output_info(),
        nodes,
        config_path: app
            .config_source
            .path()
            .map(|path| path.to_string_lossy().into_owned()),
    }
}

pub fn publish_api_events<D: crate::session::SessionDriver>(app: &mut crate::session::Session<D>) {
    if app.api_subscriptions.subscribers.is_empty() {
        return;
    }
    let snapshot = api_snapshot(app);
    app.api_subscriptions.publish(snapshot);
}

pub fn publish_config_reload<D: crate::session::SessionDriver>(
    app: &mut crate::session::Session<D>,
    accepted: bool,
) {
    app.api_subscriptions.publish_event(
        swm_ipc::EventTopic::Config,
        swm_ipc::ApiEvent::ConfigReloaded {
            sequence: 0,
            accepted,
        },
    );
}

#[cfg(test)]
mod resource_limit_tests {
    use super::*;

    #[test]
    fn connection_slots_are_bounded_and_reclaimed() {
        let limit = ConnectionLimit::default();
        let mut slots: Vec<_> = (0..MAX_CLIENTS).map(|_| limit.acquire().unwrap()).collect();
        assert!(limit.acquire().is_none());
        slots.pop();
        assert!(limit.acquire().is_some());
        drop(slots);
        assert_eq!(limit.0.load(Ordering::Acquire), 0);
    }

    #[test]
    fn disconnect_ends_a_deferred_reply_without_waiting_for_its_producer() {
        let (server, client) = UnixStream::pair().unwrap();
        let (_sender, receiver) = mpsc::channel::<()>();
        drop(client);
        assert!(wait_reply(&receiver, &server, None).is_none());
    }

    #[test]
    fn worker_disconnect_emits_cleanup_and_releases_its_slot() {
        let limit = ConnectionLimit::default();
        let (events, receiver) = channel();
        let mut event_loop = calloop::EventLoop::<Vec<u64>>::try_new().unwrap();
        event_loop
            .handle()
            .insert_source(receiver, |event, _, closed| {
                if let Event::Msg(ClientEvent::Closed(id)) = event {
                    closed.push(id);
                }
            })
            .unwrap();
        let (server, client) = UnixStream::pair().unwrap();
        drop(client);
        client_worker(
            server,
            ClientLifetime {
                id: 7,
                events,
                _slot: limit.acquire().unwrap(),
            },
        );
        let mut closed = Vec::new();
        event_loop.dispatch(Duration::ZERO, &mut closed).unwrap();
        assert_eq!(closed, [7]);
        assert_eq!(limit.0.load(Ordering::Acquire), 0);
    }
}
