use crate::flow::event::FlowEvent;
use crate::flow::{AckMode, BindEntity, Flow};
use crate::message::InboundMessage;
use crate::session::event::SessionEvent;
use crate::session::Session;
use crate::util::{
    on_flow_event_trampoline, on_flow_message_trampoline, static_no_op_on_flow_event,
};
use crate::FlowError;
use crate::SolClientReturnCode;
use solace_rs_sys as ffi;
use std::ffi::CString;
use std::marker::PhantomData;
use std::mem;

/// Builder for creating a Flow (guaranteed message consumer bound to a queue or topic endpoint).
pub struct FlowBuilder<'flow, 'session: 'flow, SM, SE, FM>
where
    SM: FnMut(InboundMessage) + Send + 'session,
    SE: FnMut(SessionEvent) + Send + 'session,
    FM: FnMut(InboundMessage) + Send + 'flow,
{
    session: &'flow Session<'session, SM, SE>,
    bind_entity: Option<BindEntity>,
    bind_name: Option<String>,
    ack_mode: AckMode,
    durable: bool,
    window_size: u32,
    on_message: Option<FM>,
    on_event: Option<Box<dyn FnMut(FlowEvent) + Send + 'flow>>,
    selector: Option<String>,
    no_local: bool,
    start_state: bool,
    max_unacked_messages: Option<i32>,
    _flow_lifetime: PhantomData<&'flow ()>,
}

impl<'flow, 'session: 'flow, SM, SE, FM> FlowBuilder<'flow, 'session, SM, SE, FM>
where
    SM: FnMut(InboundMessage) + Send + 'session,
    SE: FnMut(SessionEvent) + Send + 'session,
    FM: FnMut(InboundMessage) + Send + 'flow,
{
    pub(crate) fn new(session: &'flow Session<'session, SM, SE>) -> Self {
        FlowBuilder {
            session,
            bind_entity: None,
            bind_name: None,
            ack_mode: AckMode::Auto,
            durable: true,
            window_size: 255,
            on_message: None,
            on_event: None,
            selector: None,
            no_local: false,
            start_state: true,
            max_unacked_messages: None,
            _flow_lifetime: PhantomData,
        }
    }

    /// Set the entity type to bind to (Queue or TopicEndpoint). Required.
    pub fn bind_entity(mut self, entity: BindEntity) -> Self {
        self.bind_entity = Some(entity);
        self
    }

    /// Set the name of the queue or topic endpoint to bind to. Required.
    pub fn bind_name<T: Into<String>>(mut self, name: T) -> Self {
        self.bind_name = Some(name.into());
        self
    }

    /// Set the acknowledgement mode. Default: Auto.
    pub fn ack_mode(mut self, mode: AckMode) -> Self {
        self.ack_mode = mode;
        self
    }

    /// Set whether the endpoint is durable. Default: true.
    pub fn durable(mut self, durable: bool) -> Self {
        self.durable = durable;
        self
    }

    /// Set the flow window size. Default: 255.
    pub fn window_size(mut self, size: u32) -> Self {
        self.window_size = size;
        self
    }

    /// Set the message callback. Required.
    pub fn on_message(mut self, callback: FM) -> Self {
        self.on_message = Some(callback);
        self
    }

    /// Set the flow event callback. Optional.
    pub fn on_event(mut self, callback: impl FnMut(FlowEvent) + Send + 'flow) -> Self {
        self.on_event = Some(Box::new(callback));
        self
    }

    /// Set a selector string for filtering messages. Optional.
    pub fn selector<T: Into<String>>(mut self, selector: T) -> Self {
        self.selector = Some(selector.into());
        self
    }

    /// Set whether the flow should not receive messages published by its own session.
    pub fn no_local(mut self, no_local: bool) -> Self {
        self.no_local = no_local;
        self
    }

    /// Set whether the flow starts delivering messages immediately. Default: true.
    pub fn start_state(mut self, start: bool) -> Self {
        self.start_state = start;
        self
    }

    /// Set the maximum number of unacknowledged messages. Default: -1 (unlimited).
    pub fn max_unacked_messages(mut self, max: i32) -> Self {
        self.max_unacked_messages = Some(max);
        self
    }

    /// Build the flow and bind it to the endpoint.
    pub fn build(self) -> Result<Flow<'flow, 'session, SM, SE, FM>, FlowError> {
        let bind_entity = self
            .bind_entity
            .ok_or_else(|| FlowError::MissingRequiredArgs("bind_entity".to_string()))?;
        let bind_name = self
            .bind_name
            .ok_or_else(|| FlowError::MissingRequiredArgs("bind_name".to_string()))?;
        let on_message = self
            .on_message
            .ok_or_else(|| FlowError::MissingRequiredArgs("on_message".to_string()))?;

        let c_bind_name =
            CString::new(bind_name).map_err(FlowError::InvalidArgsNulError)?;

        // Build flow properties array
        let bind_entity_id = match bind_entity {
            BindEntity::Queue => ffi::SOLCLIENT_FLOW_PROP_BIND_ENTITY_QUEUE,
            BindEntity::TopicEndpoint => ffi::SOLCLIENT_FLOW_PROP_BIND_ENTITY_TE,
        };

        let ack_mode_val = match self.ack_mode {
            AckMode::Auto => ffi::SOLCLIENT_FLOW_PROP_ACKMODE_AUTO,
            AckMode::Client => ffi::SOLCLIENT_FLOW_PROP_ACKMODE_CLIENT,
        };

        let durable_val = if self.durable { b"1\0" } else { b"0\0" };
        let window_size_str = CString::new(self.window_size.to_string())
            .map_err(FlowError::InvalidArgsNulError)?;
        let no_local_val = if self.no_local { b"1\0" } else { b"0\0" };
        let start_state_val = if self.start_state { b"1\0" } else { b"0\0" };

        let selector_c = self
            .selector
            .as_ref()
            .map(|s| CString::new(s.as_str()))
            .transpose()
            .map_err(FlowError::InvalidArgsNulError)?;

        let max_unacked_str = self
            .max_unacked_messages
            .map(|m| CString::new(m.to_string()))
            .transpose()
            .map_err(FlowError::InvalidArgsNulError)?;

        let mut props: Vec<*const std::os::raw::c_char> = vec![
            // Bind entity type
            ffi::SOLCLIENT_FLOW_PROP_BIND_ENTITY_ID.as_ptr() as *const _,
            bind_entity_id.as_ptr() as *const _,
            // Bind name
            ffi::SOLCLIENT_FLOW_PROP_BIND_NAME.as_ptr() as *const _,
            c_bind_name.as_ptr(),
            // Durable
            ffi::SOLCLIENT_FLOW_PROP_BIND_ENTITY_DURABLE.as_ptr() as *const _,
            durable_val.as_ptr() as *const _,
            // ACK mode
            ffi::SOLCLIENT_FLOW_PROP_ACKMODE.as_ptr() as *const _,
            ack_mode_val.as_ptr() as *const _,
            // Window size
            ffi::SOLCLIENT_FLOW_PROP_WINDOWSIZE.as_ptr() as *const _,
            window_size_str.as_ptr(),
            // No local
            ffi::SOLCLIENT_FLOW_PROP_NO_LOCAL.as_ptr() as *const _,
            no_local_val.as_ptr() as *const _,
            // Start state
            ffi::SOLCLIENT_FLOW_PROP_START_STATE.as_ptr() as *const _,
            start_state_val.as_ptr() as *const _,
        ];

        // Optional selector
        if let Some(ref sel) = selector_c {
            props.push(ffi::SOLCLIENT_FLOW_PROP_SELECTOR.as_ptr() as *const _);
            props.push(sel.as_ptr());
        }

        // Optional max unacked
        if let Some(ref max) = max_unacked_str {
            props.push(ffi::SOLCLIENT_FLOW_PROP_MAX_UNACKED_MESSAGES.as_ptr() as *const _);
            props.push(max.as_ptr());
        }

        // Null terminator
        props.push(std::ptr::null());

        // Set up callback trampolines
        let mut msg_fn_box: Box<Box<FM>> = Box::new(Box::new(on_message));
        let msg_callback = on_flow_message_trampoline(&*msg_fn_box);
        let msg_user_p = &mut *msg_fn_box as *mut Box<FM> as *mut std::os::raw::c_void;

        let (event_callback, event_user_p, event_fn_box) = if let Some(on_event) = self.on_event {
            // on_event is already Box<dyn FnMut(FlowEvent) + Send + 'flow>.
            // Wrap in a second Box so the C callback receives a stable *mut Box<dyn FnMut>.
            // One deref gives F = Box<dyn FnMut(FlowEvent)> which satisfies the trampoline bound.
            let mut event_fn_box: Box<Box<dyn FnMut(FlowEvent) + Send + 'flow>> = Box::new(on_event);
            let cb = on_flow_event_trampoline(&*event_fn_box);
            let user_p = &mut *event_fn_box as *mut Box<dyn FnMut(FlowEvent) + Send + 'flow> as *mut std::os::raw::c_void;
            (cb, user_p, Some(event_fn_box))
        } else {
            (
                Some(
                    static_no_op_on_flow_event
                        as unsafe extern "C" fn(
                            ffi::solClient_opaqueFlow_pt,
                            ffi::solClient_flow_eventCallbackInfo_pt,
                            *mut std::os::raw::c_void,
                        ),
                ),
                std::ptr::null_mut(),
                None,
            )
        };

        let rx_msg_callback_info = ffi::solClient_flow_createRxMsgCallbackFuncInfo {
            callback_p: msg_callback,
            user_p: msg_user_p,
        };

        let event_callback_info = ffi::solClient_flow_createEventCallbackFuncInfo {
            callback_p: event_callback,
            user_p: event_user_p,
        };

        let mut func_info = ffi::solClient_flow_createFuncInfo_t {
            rxInfo: ffi::solClient_flow_createRxCallbackFuncInfo_t::default(),
            eventInfo: event_callback_info,
            rxMsgInfo: rx_msg_callback_info,
        };

        let mut flow_ptr: ffi::solClient_opaqueFlow_pt = std::ptr::null_mut();

        let rc = unsafe {
            ffi::solClient_session_createFlow(
                props.as_ptr() as ffi::solClient_propertyArray_pt,
                self.session._session_ptr,
                &mut flow_ptr,
                &mut func_info,
                mem::size_of::<ffi::solClient_flow_createFuncInfo_t>(),
            )
        };

        let rc = SolClientReturnCode::from_raw(rc);
        if !rc.is_ok() {
            let subcode = crate::util::get_last_error_info();
            return Err(FlowError::CreationFailure(rc, subcode));
        }

        Ok(Flow {
            _flow_ptr: flow_ptr,
            _session: self.session,
            _msg_fn_ptr: Some(msg_fn_box),
            _event_fn_ptr: event_fn_box,
            _lifetime: PhantomData,
        })
    }
}
