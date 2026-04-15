use crate::context::Context;
use crate::flow::{AckMode, FlowEvent, MessageOutcome};
use crate::message::{InboundMessage, OutboundMessage};
use crate::session::builder::SessionBuilderError;
use crate::session::event::SessionEvent;
use crate::session::Session;
use crate::util::{on_flow_event_trampoline, on_flow_message_trampoline};
use crate::{FlowError, SessionError, SolClientReturnCode};
use solace_rs_sys as ffi;
use std::mem;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{self, Poll};
use tracing::warn;

type BoxMsgFn = Box<dyn FnMut(InboundMessage) + Send + 'static>;
type BoxEventFn = Box<dyn FnMut(SessionEvent) + Send + 'static>;
type BoxFlowMsgFn = Box<dyn FnMut(InboundMessage) + Send + 'static>;
type BoxFlowEventFn = Box<dyn FnMut(FlowEvent) + Send + 'static>;

/// Shared ownership of the underlying session.
///
/// `Arc<Mutex<...>>` lets `OwnedAsyncFlow` keep the session alive through a clone
/// of this handle, decoupling flow lifetimes from session lifetimes.
type SharedSession = Arc<Mutex<Session<'static, BoxMsgFn, BoxEventFn>>>;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Error type for async session creation.
#[derive(Debug)]
pub enum AsyncSessionError {
    Builder(SessionBuilderError),
    Session(SessionError),
}

impl std::fmt::Display for AsyncSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Builder(e) => write!(f, "async session builder error: {e}"),
            Self::Session(e) => write!(f, "async session error: {e}"),
        }
    }
}

impl std::error::Error for AsyncSessionError {}

impl From<SessionBuilderError> for AsyncSessionError {
    fn from(e: SessionBuilderError) -> Self {
        Self::Builder(e)
    }
}

impl From<SessionError> for AsyncSessionError {
    fn from(e: SessionError) -> Self {
        Self::Session(e)
    }
}

// ---------------------------------------------------------------------------
// AsyncSessionBuilder
// ---------------------------------------------------------------------------

/// Builder for [`AsyncSession`].
///
/// Exposes the key `SessionBuilder` configuration options, including reconnect
/// settings that are essential for production connectors.
pub struct AsyncSessionBuilder {
    context: Context,
    host_name: Option<Vec<u8>>,
    vpn_name: Option<Vec<u8>>,
    username: Option<Vec<u8>>,
    password: Option<Vec<u8>>,
    reconnect_retries: Option<i64>,
    reconnect_retry_wait_ms: Option<u64>,
    reapply_subscriptions: Option<bool>,
    connect_timeout_ms: Option<u64>,
    ssl_trust_store_dir: Option<Vec<u8>>,
}

impl AsyncSessionBuilder {
    pub fn new(context: &Context) -> Self {
        Self {
            context: context.clone(),
            host_name: None,
            vpn_name: None,
            username: None,
            password: None,
            reconnect_retries: None,
            reconnect_retry_wait_ms: None,
            reapply_subscriptions: None,
            connect_timeout_ms: None,
            ssl_trust_store_dir: None,
        }
    }

    pub fn host_name<H: Into<Vec<u8>>>(mut self, host_name: H) -> Self {
        self.host_name = Some(host_name.into());
        self
    }

    pub fn vpn_name<V: Into<Vec<u8>>>(mut self, vpn_name: V) -> Self {
        self.vpn_name = Some(vpn_name.into());
        self
    }

    pub fn username<U: Into<Vec<u8>>>(mut self, username: U) -> Self {
        self.username = Some(username.into());
        self
    }

    pub fn password<P: Into<Vec<u8>>>(mut self, password: P) -> Self {
        self.password = Some(password.into());
        self
    }

    /// How many times to retry after a session disconnect (-1 = unlimited).
    pub fn reconnect_retries(mut self, retries: i64) -> Self {
        self.reconnect_retries = Some(retries);
        self
    }

    /// Milliseconds to wait between reconnect attempts.
    pub fn reconnect_retry_wait_ms(mut self, wait_ms: u64) -> Self {
        self.reconnect_retry_wait_ms = Some(wait_ms);
        self
    }

    /// Re-apply topic subscriptions after a reconnect.
    pub fn reapply_subscriptions(mut self, reapply: bool) -> Self {
        self.reapply_subscriptions = Some(reapply);
        self
    }

    /// Connection timeout in milliseconds.
    pub fn connect_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.connect_timeout_ms = Some(timeout_ms);
        self
    }

    /// Directory containing trusted CA certificates for TLS connections.
    pub fn ssl_trust_store_dir<S: Into<Vec<u8>>>(mut self, dir: S) -> Self {
        self.ssl_trust_store_dir = Some(dir.into());
        self
    }

    pub fn build(self) -> Result<AsyncSession, AsyncSessionError> {
        let (msg_tx, msg_rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();

        let on_message: BoxMsgFn = Box::new(move |msg| {
            let _ = msg_tx.send(msg);
        });
        let on_event: BoxEventFn = Box::new(move |event| {
            let _ = event_tx.send(event);
        });

        let missing = |field: &'static str| {
            AsyncSessionError::Builder(SessionBuilderError::MissingRequiredArgs(field.to_string()))
        };

        let mut builder = self
            .context
            .session_builder::<Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, BoxMsgFn, BoxEventFn>()
            .host_name(self.host_name.ok_or_else(|| missing("host_name"))?)
            .vpn_name(self.vpn_name.ok_or_else(|| missing("vpn_name"))?)
            .username(self.username.ok_or_else(|| missing("username"))?)
            .password(self.password.ok_or_else(|| missing("password"))?)
            .on_message(on_message)
            .on_event(on_event);

        if let Some(retries) = self.reconnect_retries {
            builder = builder.reconnect_retries(retries);
        }
        if let Some(wait_ms) = self.reconnect_retry_wait_ms {
            builder = builder.reconnect_retry_wait_ms(wait_ms);
        }
        if let Some(reapply) = self.reapply_subscriptions {
            builder = builder.reapply_subscriptions(reapply);
        }
        if let Some(timeout_ms) = self.connect_timeout_ms {
            builder = builder.connect_timeout_ms(timeout_ms);
        }
        if let Some(dir) = self.ssl_trust_store_dir {
            builder = builder.ssl_trust_store_dir(dir);
        }

        let inner = builder.build()?;

        Ok(AsyncSession {
            inner: Arc::new(Mutex::new(inner)),
            msg_rx,
            event_rx,
        })
    }
}

// ---------------------------------------------------------------------------
// AsyncSession
// ---------------------------------------------------------------------------

/// An async wrapper around `Session` that bridges the callback model to async channels.
///
/// Messages and events arrive via `tokio::sync::mpsc` channels instead of closures,
/// enabling `async fn recv()` and `Stream` consumption.
///
/// The session is stored behind `Arc<Mutex<...>>` so that [`OwnedAsyncFlow`] instances
/// keep the session alive independently — this eliminates the lifetime coupling that
/// would otherwise prevent storing both a session and its flows in the same struct
/// (e.g. a `SolaceSource` in a RisingWave connector).
pub struct AsyncSession {
    inner: SharedSession,
    msg_rx: tokio::sync::mpsc::UnboundedReceiver<InboundMessage>,
    event_rx: tokio::sync::mpsc::UnboundedReceiver<SessionEvent>,
}

impl AsyncSession {
    /// Create a new async session connected to the broker.
    ///
    /// For more control over reconnect and TLS settings use [`AsyncSessionBuilder`].
    pub fn new<H, V, U, P>(
        context: &Context,
        host_name: H,
        vpn_name: V,
        username: U,
        password: P,
    ) -> Result<Self, AsyncSessionError>
    where
        H: Into<Vec<u8>>,
        V: Into<Vec<u8>>,
        U: Into<Vec<u8>>,
        P: Into<Vec<u8>>,
    {
        AsyncSessionBuilder::new(context)
            .host_name(host_name)
            .vpn_name(vpn_name)
            .username(username)
            .password(password)
            .build()
    }

    /// Receive the next message asynchronously.
    pub async fn recv(&mut self) -> Option<InboundMessage> {
        self.msg_rx.recv().await
    }

    /// Attempt to receive a message without blocking.
    pub fn try_recv(&mut self) -> Result<InboundMessage, tokio::sync::mpsc::error::TryRecvError> {
        self.msg_rx.try_recv()
    }

    /// Receive the next session event asynchronously.
    pub async fn recv_event(&mut self) -> Option<SessionEvent> {
        self.event_rx.recv().await
    }

    /// Publish a message (synchronous — the Solace C API publish is blocking).
    pub fn publish(&self, message: OutboundMessage) -> Result<(), SessionError> {
        self.inner.lock().unwrap().publish(message)
    }

    /// Subscribe to a topic.
    pub fn subscribe<T: Into<Vec<u8>>>(&self, topic: T) -> Result<(), SessionError> {
        self.inner.lock().unwrap().subscribe(topic)
    }

    /// Unsubscribe from a topic.
    pub fn unsubscribe<T: Into<Vec<u8>>>(&self, topic: T) -> Result<(), SessionError> {
        self.inner.lock().unwrap().unsubscribe(topic)
    }

    /// Create an async flow bound to a queue on this session.
    ///
    /// Returns an [`OwnedAsyncFlow`] that is `'static` and independently movable —
    /// it can be stored in the same struct as `AsyncSession` without lifetime conflicts.
    ///
    /// The flow holds an `Arc` clone of the session, so the session will not be freed
    /// until all flows derived from it have been dropped.
    pub fn create_flow(
        &self,
        queue_name: &str,
        ack_mode: AckMode,
    ) -> Result<OwnedAsyncFlow, FlowError> {
        let (msg_tx, msg_rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();

        let on_message: BoxFlowMsgFn = Box::new(move |msg: InboundMessage| {
            let _ = msg_tx.send(msg);
        });
        let on_event: BoxFlowEventFn = Box::new(move |event: FlowEvent| {
            let _ = event_tx.send(event);
        });

        // Build the flow properties array.
        let c_bind_name =
            std::ffi::CString::new(queue_name).map_err(FlowError::InvalidArgsNulError)?;
        let bind_entity_id = ffi::SOLCLIENT_FLOW_PROP_BIND_ENTITY_QUEUE;
        let ack_mode_val = match ack_mode {
            AckMode::Auto => ffi::SOLCLIENT_FLOW_PROP_ACKMODE_AUTO,
            AckMode::Client => ffi::SOLCLIENT_FLOW_PROP_ACKMODE_CLIENT,
        };
        let durable_val: &[u8] = b"1\0";
        let window_size_str =
            std::ffi::CString::new("255").map_err(FlowError::InvalidArgsNulError)?;
        let start_state_val: &[u8] = b"1\0";

        let props: Vec<*const std::os::raw::c_char> = vec![
            ffi::SOLCLIENT_FLOW_PROP_BIND_ENTITY_ID.as_ptr() as *const _,
            bind_entity_id.as_ptr() as *const _,
            ffi::SOLCLIENT_FLOW_PROP_BIND_NAME.as_ptr() as *const _,
            c_bind_name.as_ptr(),
            ffi::SOLCLIENT_FLOW_PROP_BIND_ENTITY_DURABLE.as_ptr() as *const _,
            durable_val.as_ptr() as *const _,
            ffi::SOLCLIENT_FLOW_PROP_ACKMODE.as_ptr() as *const _,
            ack_mode_val.as_ptr() as *const _,
            ffi::SOLCLIENT_FLOW_PROP_WINDOWSIZE.as_ptr() as *const _,
            window_size_str.as_ptr(),
            ffi::SOLCLIENT_FLOW_PROP_START_STATE.as_ptr() as *const _,
            start_state_val.as_ptr() as *const _,
            std::ptr::null(),
        ];

        // Set up callback trampolines.
        // The double-boxing pattern mirrors FlowBuilder::build(): the outer Box gives us a stable
        // heap address for the inner Box<dyn FnMut> fat pointer, which is what the C callback
        // receives as user_p.
        let mut msg_fn_box: Box<Box<BoxFlowMsgFn>> = Box::new(Box::new(on_message));
        // Deref two levels so F = BoxFlowMsgFn (not Box<BoxFlowMsgFn>), matching
        // the sync FlowBuilder pattern where user_p points to Box<F>.
        let msg_callback = on_flow_message_trampoline(&**msg_fn_box);
        let msg_user_p = &mut *msg_fn_box as *mut Box<BoxFlowMsgFn> as *mut std::os::raw::c_void;

        let mut event_fn_box: Box<Box<BoxFlowEventFn>> = Box::new(Box::new(on_event));
        let event_callback = on_flow_event_trampoline(&**event_fn_box);
        let event_user_p =
            &mut *event_fn_box as *mut Box<BoxFlowEventFn> as *mut std::os::raw::c_void;

        let rx_msg_callback_info = ffi::solClient_flow_createRxMsgCallbackFuncInfo {
            callback_p: msg_callback,
            user_p: msg_user_p,
        };
        let event_callback_info = ffi::solClient_flow_createEventCallbackFuncInfo {
            callback_p: event_callback,
            user_p: event_user_p,
        };
        let mut func_info = ffi::solClient_flow_createFuncInfo_t {
            rxInfo: unsafe { mem::zeroed() },
            eventInfo: event_callback_info,
            rxMsgInfo: rx_msg_callback_info,
        };

        let mut flow_ptr: ffi::solClient_opaqueFlow_pt = std::ptr::null_mut();

        // Lock the session only for the duration of the C call, then release immediately.
        let rc = {
            let session_guard = self.inner.lock().unwrap();
            unsafe {
                ffi::solClient_session_createFlow(
                    props.as_ptr() as *mut *mut _,
                    session_guard._session_ptr,
                    &mut flow_ptr,
                    &mut func_info,
                    mem::size_of::<ffi::solClient_flow_createFuncInfo_t>(),
                )
            }
        };

        let rc = SolClientReturnCode::from_raw(rc);
        if !rc.is_ok() {
            let subcode = crate::util::get_last_error_info();
            return Err(FlowError::CreationFailure(rc, subcode));
        }

        Ok(OwnedAsyncFlow {
            flow_ptr,
            _session_guard: Arc::clone(&self.inner),
            _msg_fn_ptr: msg_fn_box,
            _event_fn_ptr: event_fn_box,
            msg_rx,
            _event_rx: event_rx,
        })
    }

    /// Disconnect the session.
    ///
    /// All [`OwnedAsyncFlow`] instances derived from this session must be dropped
    /// before calling this method. Returns an error if any flows still hold a
    /// reference to the session.
    pub fn disconnect(self) -> Result<(), SessionError> {
        match Arc::try_unwrap(self.inner) {
            Ok(mutex) => mutex
                .into_inner()
                .unwrap_or_else(|e| e.into_inner())
                .disconnect(),
            Err(_) => Err(SessionError::DisconnectError(
                SolClientReturnCode::Fail,
                crate::SolClientSubCode {
                    subcode: 0,
                    error_string: "cannot disconnect: drop all OwnedAsyncFlow instances first"
                        .to_string(),
                },
            )),
        }
    }
}

impl futures_core::Stream for AsyncSession {
    type Item = InboundMessage;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        self.msg_rx.poll_recv(cx)
    }
}

// ---------------------------------------------------------------------------
// OwnedAsyncFlow
// ---------------------------------------------------------------------------

/// An async wrapper around a guaranteed message flow that is `'static` and independently owned.
///
/// Unlike the former `AsyncFlow<'_, ...>`, `OwnedAsyncFlow` holds an `Arc` clone of the
/// session rather than a lifetime borrow, making it freely movable between structs and
/// threads. This is the key enabler for RisingWave-style connectors that need to store
/// both a session and a flow in the same struct:
///
/// ```ignore
/// struct SolaceSource {
///     session: AsyncSession,
///     flow: OwnedAsyncFlow,
/// }
/// ```
///
/// Drop ordering is guaranteed by the `Arc` refcount: the session is only freed after
/// every `OwnedAsyncFlow` that references it has been dropped.
pub struct OwnedAsyncFlow {
    flow_ptr: ffi::solClient_opaqueFlow_pt,
    /// Keeps the session alive for at least as long as this flow exists.
    _session_guard: SharedSession,
    /// Keeps the message callback closure alive; address must not change after creation.
    #[allow(clippy::redundant_allocation)]
    _msg_fn_ptr: Box<Box<BoxFlowMsgFn>>,
    /// Keeps the event callback closure alive.
    #[allow(dead_code, clippy::redundant_allocation)]
    _event_fn_ptr: Box<Box<BoxFlowEventFn>>,
    msg_rx: tokio::sync::mpsc::UnboundedReceiver<InboundMessage>,
    _event_rx: tokio::sync::mpsc::UnboundedReceiver<FlowEvent>,
}

// Safety: `flow_ptr` is a C opaque pointer accessed only through &self/&mut self.
// The `Arc` session guard ensures the session outlives the flow. The boxed closures
// are only called from the Solace context thread.
unsafe impl Send for OwnedAsyncFlow {}

impl OwnedAsyncFlow {
    /// Receive the next guaranteed message asynchronously.
    pub async fn recv(&mut self) -> Option<InboundMessage> {
        self.msg_rx.recv().await
    }

    /// Attempt to receive a message without blocking.
    pub fn try_recv(&mut self) -> Result<InboundMessage, tokio::sync::mpsc::error::TryRecvError> {
        self.msg_rx.try_recv()
    }

    /// Acknowledge a guaranteed message by its message ID.
    ///
    /// Use in `AckMode::Client` after successfully processing a message.
    pub fn ack(&self, msg_id: u64) -> Result<(), FlowError> {
        let rc = unsafe { ffi::solClient_flow_sendAck(self.flow_ptr, msg_id) };
        let rc = SolClientReturnCode::from_raw(rc);
        if !rc.is_ok() {
            let subcode = crate::util::get_last_error_info();
            return Err(FlowError::AckFailure(msg_id, rc, subcode));
        }
        Ok(())
    }

    /// Settle a guaranteed message with a specific outcome.
    ///
    /// This is an alternative to `ack()` that allows specifying whether the message
    /// was accepted, failed (redelivered), or rejected (moved to DMQ).
    pub fn settle(&self, msg_id: u64, outcome: MessageOutcome) -> Result<(), FlowError> {
        let rc = unsafe { ffi::solClient_flow_settleMsg(self.flow_ptr, msg_id, outcome.to_ffi()) };
        let rc = SolClientReturnCode::from_raw(rc);
        if !rc.is_ok() {
            let subcode = crate::util::get_last_error_info();
            return Err(FlowError::SettleFailure(msg_id, rc, subcode));
        }
        Ok(())
    }

    /// Start message delivery on this flow.
    pub fn start(&self) -> Result<(), FlowError> {
        let rc = unsafe { ffi::solClient_flow_start(self.flow_ptr) };
        let rc = SolClientReturnCode::from_raw(rc);
        if !rc.is_ok() {
            let subcode = crate::util::get_last_error_info();
            return Err(FlowError::StartFailure(rc, subcode));
        }
        Ok(())
    }

    /// Stop message delivery on this flow. Messages remain queued on the broker.
    pub fn stop(&self) -> Result<(), FlowError> {
        let rc = unsafe { ffi::solClient_flow_stop(self.flow_ptr) };
        let rc = SolClientReturnCode::from_raw(rc);
        if !rc.is_ok() {
            let subcode = crate::util::get_last_error_info();
            return Err(FlowError::StopFailure(rc, subcode));
        }
        Ok(())
    }
}

impl Drop for OwnedAsyncFlow {
    fn drop(&mut self) {
        let rc = unsafe { ffi::solClient_flow_destroy(&mut self.flow_ptr) };
        let rc = SolClientReturnCode::from_raw(rc);
        if !rc.is_ok() {
            warn!("OwnedAsyncFlow was not dropped properly: {rc}");
        }
    }
}

impl futures_core::Stream for OwnedAsyncFlow {
    type Item = InboundMessage;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        self.msg_rx.poll_recv(cx)
    }
}
