use core::fmt;
use enum_primitive::*;
use solace_rs_sys as ffi;
use std::ffi::CStr;

enum_from_primitive! {
    #[derive(Debug, PartialEq, Eq, Copy, Clone)]
    #[repr(u32)]
    pub enum FlowEvent {
        UpNotice=ffi::solClient_flow_event_SOLCLIENT_FLOW_EVENT_UP_NOTICE,
        DownError=ffi::solClient_flow_event_SOLCLIENT_FLOW_EVENT_DOWN_ERROR,
        BindFailedError=ffi::solClient_flow_event_SOLCLIENT_FLOW_EVENT_BIND_FAILED_ERROR,
        RejectedMsgError=ffi::solClient_flow_event_SOLCLIENT_FLOW_EVENT_REJECTED_MSG_ERROR,
        SessionDown=ffi::solClient_flow_event_SOLCLIENT_FLOW_EVENT_SESSION_DOWN,
        Active=ffi::solClient_flow_event_SOLCLIENT_FLOW_EVENT_ACTIVE,
        Inactive=ffi::solClient_flow_event_SOLCLIENT_FLOW_EVENT_INACTIVE,
        Reconnecting=ffi::solClient_flow_event_SOLCLIENT_FLOW_EVENT_RECONNECTING,
        Reconnected=ffi::solClient_flow_event_SOLCLIENT_FLOW_EVENT_RECONNECTED,
    }
}

impl fmt::Display for FlowEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let raw_event = *self as u32 as std::os::raw::c_uint;
        let raw_c_ptr = unsafe { ffi::solClient_flow_eventToString(raw_event) };
        let c_str = unsafe { CStr::from_ptr(raw_c_ptr) };
        let message = c_str.to_str().unwrap_or("Unknown Flow Event");
        write!(f, "{}", message)
    }
}
