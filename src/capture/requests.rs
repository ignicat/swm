//! Start and validate compositor source-chooser requests.

use super::*;

pub fn request_source<D: SessionDriver>(
    session: &mut Session<D>,
    request: swm_ipc::SourceChooserRequest,
    reply: crate::ipc::ReplySender,
) {
    if session.session_lock.active() {
        let _ = reply.send(
            swm_ipc::Response::Source(swm_ipc::SourceChooserResponse::Failed {
                message: "session is locked".to_string(),
            }),
            Vec::new(),
        );
        return;
    }
    let supported = request.source_types & (swm_ipc::SOURCE_MONITOR | swm_ipc::SOURCE_WINDOW);
    if supported == 0 {
        let _ = reply.send(
            swm_ipc::Response::Source(swm_ipc::SourceChooserResponse::Failed {
                message: "no supported source type was requested".to_string(),
            }),
            Vec::new(),
        );
        return;
    }
    let pending = PendingSourceChooser {
        request_handle: request.request_handle,
        reply,
    };
    let preferred =
        crate::wayland::focus::selected_output(&session.wayland).map(|output| output.name());
    if let Err(PendingSourceChooser { reply, .. }) = session.capture.begin_source(
        &session.wayland.space,
        preferred.as_deref(),
        supported,
        pending,
    ) {
        let _ = reply.send(
            swm_ipc::Response::Source(swm_ipc::SourceChooserResponse::Failed {
                message: "another source chooser is already active".to_string(),
            }),
            Vec::new(),
        );
        return;
    }
    begin_modal_capture(session);
}

pub fn cancel_portal<D: SessionDriver>(session: &mut Session<D>, request_handle: &str) -> bool {
    let matches = matches!(
        session.capture.pending.as_ref(),
        Some(PendingSourceChooser { request_handle: active, .. }) if active == request_handle
    );
    if !matches {
        return false;
    }
    if let Some(PendingSourceChooser { reply, .. }) = session.capture.cancel() {
        let _ = reply.send(
            swm_ipc::Response::Source(swm_ipc::SourceChooserResponse::Cancelled),
            Vec::new(),
        );
    }
    finish_modal_capture(session);
    true
}
