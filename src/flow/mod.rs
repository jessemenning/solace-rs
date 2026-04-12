pub mod builder;
pub mod event;

pub use builder::FlowBuilder;
pub use event::FlowEvent;
// Flow is public but its generic signature is complex;
// users typically interact through FlowBuilder.

use crate::message::InboundMessage;
use crate::session::event::SessionEvent;
use crate::session::Session;
use crate::util::get_last_error_info;
use crate::{FlowError, SolClientReturnCode};
use solace_rs_sys as ffi;
use std::marker::PhantomData;
use tracing::warn;

/// Controls whether messages are automatically acknowledged by the API
/// or require explicit client acknowledgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckMode {
    /// Messages are auto-acknowledged after the callback returns.
    Auto,
    /// Application must explicitly call `ack()` or `settle()`.
    Client,
}

/// What type of endpoint to bind the flow to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindEntity {
    /// Bind to a queue endpoint.
    Queue,
    /// Bind to a topic endpoint.
    TopicEndpoint,
}

/// Outcome for settling a message in Client ACK mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageOutcome {
    Accepted,
    Failed,
    Rejected,
}

impl MessageOutcome {
    pub fn to_ffi(self) -> ffi::solClient_msgOutcome_t {
        match self {
            MessageOutcome::Accepted => ffi::solClient_msgOutcome_SOLCLIENT_OUTCOME_ACCEPTED,
            MessageOutcome::Failed => ffi::solClient_msgOutcome_SOLCLIENT_OUTCOME_FAILED,
            MessageOutcome::Rejected => ffi::solClient_msgOutcome_SOLCLIENT_OUTCOME_REJECTED,
        }
    }
}

/// A guaranteed message consumer flow bound to a queue or topic endpoint.
///
/// Created via `Session::flow_builder()`. The flow holds a borrow of the session
/// to ensure the session outlives all its flows.
pub struct Flow<
    'flow,
    'session: 'flow,
    SM: FnMut(InboundMessage) + Send + 'session,
    SE: FnMut(SessionEvent) + Send + 'session,
    FM: FnMut(InboundMessage) + Send + 'flow,
> {
    pub(crate) _flow_ptr: ffi::solClient_opaqueFlow_pt,
    #[allow(dead_code)]
    pub(crate) _session: &'flow Session<'session, SM, SE>,
    #[allow(dead_code, clippy::redundant_allocation)]
    pub(crate) _msg_fn_ptr: Option<Box<Box<FM>>>,
    #[allow(dead_code, clippy::redundant_allocation)]
    pub(crate) _event_fn_ptr: Option<Box<Box<dyn FnMut(FlowEvent) + Send + 'flow>>>,
    pub(crate) _lifetime: PhantomData<&'flow ()>,
}

unsafe impl<SM, SE, FM> Send for Flow<'_, '_, SM, SE, FM>
where
    SM: FnMut(InboundMessage) + Send,
    SE: FnMut(SessionEvent) + Send,
    FM: FnMut(InboundMessage) + Send,
{
}

impl<'flow, 'session: 'flow, SM, SE, FM> Flow<'flow, 'session, SM, SE, FM>
where
    SM: FnMut(InboundMessage) + Send + 'session,
    SE: FnMut(SessionEvent) + Send + 'session,
    FM: FnMut(InboundMessage) + Send + 'flow,
{
    /// Acknowledge a guaranteed message by its message ID.
    ///
    /// Use this in Client ACK mode after successfully processing a message.
    /// Call `InboundMessage::get_msg_id()` to obtain the ID.
    pub fn ack(&self, msg_id: u64) -> Result<(), FlowError> {
        let rc = unsafe { ffi::solClient_flow_sendAck(self._flow_ptr, msg_id) };

        let rc = SolClientReturnCode::from_raw(rc);
        if !rc.is_ok() {
            let subcode = get_last_error_info();
            return Err(FlowError::AckFailure(msg_id, rc, subcode));
        }
        Ok(())
    }

    /// Settle a guaranteed message with a specific outcome.
    ///
    /// This is an alternative to `ack()` that allows specifying whether the message
    /// was accepted, failed (redelivered), or rejected (moved to DMQ).
    pub fn settle(&self, msg_id: u64, outcome: MessageOutcome) -> Result<(), FlowError> {
        let rc = unsafe {
            ffi::solClient_flow_settleMsg(self._flow_ptr, msg_id, outcome.to_ffi())
        };

        let rc = SolClientReturnCode::from_raw(rc);
        if !rc.is_ok() {
            let subcode = get_last_error_info();
            return Err(FlowError::SettleFailure(msg_id, rc, subcode));
        }
        Ok(())
    }

    /// Start message delivery on this flow.
    pub fn start(&self) -> Result<(), FlowError> {
        let rc = unsafe { ffi::solClient_flow_start(self._flow_ptr) };

        let rc = SolClientReturnCode::from_raw(rc);
        if !rc.is_ok() {
            let subcode = get_last_error_info();
            return Err(FlowError::StartFailure(rc, subcode));
        }
        Ok(())
    }

    /// Stop message delivery on this flow. Messages remain queued on the broker.
    pub fn stop(&self) -> Result<(), FlowError> {
        let rc = unsafe { ffi::solClient_flow_stop(self._flow_ptr) };

        let rc = SolClientReturnCode::from_raw(rc);
        if !rc.is_ok() {
            let subcode = get_last_error_info();
            return Err(FlowError::StopFailure(rc, subcode));
        }
        Ok(())
    }
}

impl<SM, SE, FM> Drop for Flow<'_, '_, SM, SE, FM>
where
    SM: FnMut(InboundMessage) + Send,
    SE: FnMut(SessionEvent) + Send,
    FM: FnMut(InboundMessage) + Send,
{
    fn drop(&mut self) {
        let rc = unsafe { ffi::solClient_flow_destroy(&mut self._flow_ptr) };
        let rc = SolClientReturnCode::from_raw(rc);
        if !rc.is_ok() {
            warn!("flow was not dropped properly. {rc}");
        }
    }
}
