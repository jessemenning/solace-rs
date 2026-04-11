use ffi::solClient_getLastErrorInfo;
use num_traits::FromPrimitive;

use crate::flow::FlowEvent;
use crate::message::InboundMessage;
use crate::session::SessionEvent;
use crate::SolClientSubCode;
use solace_rs_sys as ffi;
use std::panic;

pub(crate) fn on_message_trampoline<'s, F>(
    _closure: &'s F,
) -> ffi::solClient_session_rxMsgCallbackFunc_t
where
    F: FnMut(InboundMessage) + Send + 's,
{
    Some(static_on_message::<F>)
}

pub(crate) fn on_event_trampoline<'s, F>(
    _closure: &'s F,
) -> ffi::solClient_session_eventCallbackFunc_t
where
    F: FnMut(SessionEvent) + Send + 's,
{
    Some(static_on_event::<F>)
}

pub(crate) extern "C" fn static_no_op_on_message(
    _opaque_session_p: ffi::solClient_opaqueSession_pt,
    _msg_p: ffi::solClient_opaqueMsg_pt,
    _raw_user_closure: *mut ::std::os::raw::c_void,
) -> ffi::solClient_rxMsgCallback_returnCode_t {
    ffi::solClient_rxMsgCallback_returnCode_SOLCLIENT_CALLBACK_OK
}

extern "C" fn static_on_message<'s, F>(
    _opaque_session_p: ffi::solClient_opaqueSession_pt, // non-null
    msg_p: ffi::solClient_opaqueMsg_pt,                 // non-null
    raw_user_closure: *mut ::std::os::raw::c_void,      // can be null
) -> ffi::solClient_rxMsgCallback_returnCode_t
where
    F: FnMut(InboundMessage) + Send + 's,
{
    let non_null_raw_user_closure = std::ptr::NonNull::new(raw_user_closure);

    let Some(raw_user_closure) = non_null_raw_user_closure else {
        return ffi::solClient_rxMsgCallback_returnCode_SOLCLIENT_CALLBACK_OK;
    };

    // Transfer ownership of the message to InboundMessage; TAKE_MSG must always
    // be returned after this point so the C library does not double-free.
    let message = InboundMessage::from(msg_p);
    let user_closure: &mut Box<F> =
        unsafe { &mut *(raw_user_closure.as_ptr() as *mut Box<F>) };
    let _ = panic::catch_unwind(panic::AssertUnwindSafe(|| user_closure(message)));

    ffi::solClient_rxMsgCallback_returnCode_SOLCLIENT_CALLBACK_TAKE_MSG
}

pub(crate) extern "C" fn static_no_op_on_event(
    _opaque_session_p: ffi::solClient_opaqueSession_pt, // non-null
    _event_info_p: ffi::solClient_session_eventCallbackInfo_pt, //non-null
    _raw_user_closure: *mut ::std::os::raw::c_void,     // can be null
) {
}

extern "C" fn static_on_event<'s, F>(
    _opaque_session_p: ffi::solClient_opaqueSession_pt, // non-null
    event_info_p: ffi::solClient_session_eventCallbackInfo_pt, //non-null
    raw_user_closure: *mut ::std::os::raw::c_void,      // can be null
) where
    F: FnMut(SessionEvent) + Send + 's,
{
    let non_null_raw_user_closure = std::ptr::NonNull::new(raw_user_closure);

    let Some(raw_user_closure) = non_null_raw_user_closure else {
        return;
    };
    let raw_event = unsafe { (*event_info_p).sessionEvent };

    let Some(event) = SessionEvent::from_u32(raw_event) else {
        return;
    };

    let user_closure: &mut Box<F> =
        unsafe { &mut *(raw_user_closure.as_ptr() as *mut Box<F>) };
    let _ = panic::catch_unwind(panic::AssertUnwindSafe(|| user_closure(event)));
}

// --- Flow callback trampolines ---

pub(crate) fn on_flow_message_trampoline<'s, F>(
    _closure: &'s F,
) -> ffi::solClient_flow_rxMsgCallbackFunc_t
where
    F: FnMut(InboundMessage) + Send + 's,
{
    Some(static_on_flow_message::<F>)
}

pub(crate) fn on_flow_event_trampoline<'s, F>(
    _closure: &'s F,
) -> ffi::solClient_flow_eventCallbackFunc_t
where
    F: FnMut(FlowEvent) + Send + 's,
{
    Some(static_on_flow_event::<F>)
}

#[allow(dead_code)]
pub(crate) extern "C" fn static_no_op_on_flow_message(
    _opaque_flow_p: ffi::solClient_opaqueFlow_pt,
    _msg_p: ffi::solClient_opaqueMsg_pt,
    _raw_user_closure: *mut ::std::os::raw::c_void,
) -> ffi::solClient_rxMsgCallback_returnCode_t {
    ffi::solClient_rxMsgCallback_returnCode_SOLCLIENT_CALLBACK_OK
}

extern "C" fn static_on_flow_message<'s, F>(
    _opaque_flow_p: ffi::solClient_opaqueFlow_pt,
    msg_p: ffi::solClient_opaqueMsg_pt,
    raw_user_closure: *mut ::std::os::raw::c_void,
) -> ffi::solClient_rxMsgCallback_returnCode_t
where
    F: FnMut(InboundMessage) + Send + 's,
{
    let non_null_raw_user_closure = std::ptr::NonNull::new(raw_user_closure);

    let Some(raw_user_closure) = non_null_raw_user_closure else {
        return ffi::solClient_rxMsgCallback_returnCode_SOLCLIENT_CALLBACK_OK;
    };

    // Transfer ownership of the message to InboundMessage; TAKE_MSG must always
    // be returned after this point so the C library does not double-free.
    let message = InboundMessage::from(msg_p);
    let user_closure: &mut Box<F> =
        unsafe { &mut *(raw_user_closure.as_ptr() as *mut Box<F>) };
    let _ = panic::catch_unwind(panic::AssertUnwindSafe(|| user_closure(message)));

    ffi::solClient_rxMsgCallback_returnCode_SOLCLIENT_CALLBACK_TAKE_MSG
}

pub(crate) extern "C" fn static_no_op_on_flow_event(
    _opaque_flow_p: ffi::solClient_opaqueFlow_pt,
    _event_info_p: ffi::solClient_flow_eventCallbackInfo_pt,
    _raw_user_closure: *mut ::std::os::raw::c_void,
) {
}

extern "C" fn static_on_flow_event<'s, F>(
    _opaque_flow_p: ffi::solClient_opaqueFlow_pt,
    event_info_p: ffi::solClient_flow_eventCallbackInfo_pt,
    raw_user_closure: *mut ::std::os::raw::c_void,
) where
    F: FnMut(FlowEvent) + Send + 's,
{
    let non_null_raw_user_closure = std::ptr::NonNull::new(raw_user_closure);

    let Some(raw_user_closure) = non_null_raw_user_closure else {
        return;
    };
    let raw_event = unsafe { (*event_info_p).flowEvent };

    let Some(event) = FlowEvent::from_u32(raw_event) else {
        return;
    };

    let user_closure: &mut Box<F> =
        unsafe { &mut *(raw_user_closure.as_ptr() as *mut Box<F>) };
    let _ = panic::catch_unwind(panic::AssertUnwindSafe(|| user_closure(event)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SolClientReturnCode;
    use solace_rs_sys as ffi;
    use std::ptr;

    // --- Helpers ---

    /// Allocate a real Solace message.  solClient_msg_alloc works without a
    /// running context and is safe to pair with InboundMessage (which calls
    /// solClient_msg_free on drop).
    fn alloc_msg() -> ffi::solClient_opaqueMsg_pt {
        let mut p: ffi::solClient_opaqueMsg_pt = ptr::null_mut();
        let rc = unsafe { ffi::solClient_msg_alloc(&mut p) };
        assert_eq!(
            SolClientReturnCode::from_raw(rc),
            SolClientReturnCode::Ok,
            "solClient_msg_alloc failed"
        );
        p
    }

    /// Wrap a concrete `fn(InboundMessage)` in the same double-box layout that
    /// the session builder uses.  Returns (raw_cookie, owner_box).
    /// Caller must keep `owner_box` alive for the duration of the callback.
    fn wrap_msg_fn(
        f: fn(InboundMessage),
    ) -> (*mut ::std::os::raw::c_void, Box<Box<fn(InboundMessage)>>) {
        let mut outer: Box<Box<fn(InboundMessage)>> = Box::new(Box::new(f));
        let raw = outer.as_mut() as *mut _ as *mut ::std::os::raw::c_void;
        (raw, outer)
    }

    /// Same for `fn(SessionEvent)`.
    fn wrap_event_fn(
        f: fn(SessionEvent),
    ) -> (*mut ::std::os::raw::c_void, Box<Box<fn(SessionEvent)>>) {
        let mut outer: Box<Box<fn(SessionEvent)>> = Box::new(Box::new(f));
        let raw = outer.as_mut() as *mut _ as *mut ::std::os::raw::c_void;
        (raw, outer)
    }

    /// Same for `fn(FlowEvent)`.
    fn wrap_flow_event_fn(
        f: fn(FlowEvent),
    ) -> (*mut ::std::os::raw::c_void, Box<Box<fn(FlowEvent)>>) {
        let mut outer: Box<Box<fn(FlowEvent)>> = Box::new(Box::new(f));
        let raw = outer.as_mut() as *mut _ as *mut ::std::os::raw::c_void;
        (raw, outer)
    }

    fn make_session_event_info() -> ffi::solClient_session_eventCallbackInfo {
        ffi::solClient_session_eventCallbackInfo {
            sessionEvent: ffi::solClient_session_event_SOLCLIENT_SESSION_EVENT_UP_NOTICE,
            responseCode: 0,
            info_p: ptr::null(),
            correlation_p: ptr::null_mut(),
        }
    }

    fn make_flow_event_info() -> ffi::solClient_flow_eventCallbackInfo {
        ffi::solClient_flow_eventCallbackInfo {
            flowEvent: ffi::solClient_flow_event_SOLCLIENT_FLOW_EVENT_UP_NOTICE,
            responseCode: 0,
            info_p: ptr::null(),
        }
    }

    // --- static_on_message ---

    #[test]
    fn on_message_null_closure_returns_callback_ok() {
        // When the user cookie is null the callback must return CALLBACK_OK
        // without touching msg_p.
        let rc = static_on_message::<fn(InboundMessage)>(
            ptr::null_mut(),
            ptr::null_mut(), // msg_p irrelevant – early return before use
            ptr::null_mut(), // null cookie
        );
        assert_eq!(
            rc,
            ffi::solClient_rxMsgCallback_returnCode_SOLCLIENT_CALLBACK_OK
        );
    }

    #[test]
    fn on_message_normal_fn_returns_callback_take_msg() {
        fn noop(_: InboundMessage) {}
        let msg_p = alloc_msg();
        let (raw, _owner) = wrap_msg_fn(noop);
        let rc = static_on_message::<fn(InboundMessage)>(ptr::null_mut(), msg_p, raw);
        assert_eq!(
            rc,
            ffi::solClient_rxMsgCallback_returnCode_SOLCLIENT_CALLBACK_TAKE_MSG
        );
        // InboundMessage::drop called solClient_msg_free; _owner still valid.
    }

    #[test]
    fn on_message_panicking_fn_returns_callback_take_msg() {
        // The panic must be caught by catch_unwind; TAKE_MSG still returned.
        fn panicking(_: InboundMessage) {
            panic!("intentional panic – must be caught");
        }
        let msg_p = alloc_msg();
        let (raw, _owner) = wrap_msg_fn(panicking);
        let rc = static_on_message::<fn(InboundMessage)>(ptr::null_mut(), msg_p, raw);
        assert_eq!(
            rc,
            ffi::solClient_rxMsgCallback_returnCode_SOLCLIENT_CALLBACK_TAKE_MSG
        );
    }

    // --- static_on_flow_message ---

    #[test]
    fn on_flow_message_null_closure_returns_callback_ok() {
        let rc = static_on_flow_message::<fn(InboundMessage)>(
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        );
        assert_eq!(
            rc,
            ffi::solClient_rxMsgCallback_returnCode_SOLCLIENT_CALLBACK_OK
        );
    }

    #[test]
    fn on_flow_message_normal_fn_returns_callback_take_msg() {
        fn noop(_: InboundMessage) {}
        let msg_p = alloc_msg();
        let (raw, _owner) = wrap_msg_fn(noop);
        let rc = static_on_flow_message::<fn(InboundMessage)>(ptr::null_mut(), msg_p, raw);
        assert_eq!(
            rc,
            ffi::solClient_rxMsgCallback_returnCode_SOLCLIENT_CALLBACK_TAKE_MSG
        );
    }

    #[test]
    fn on_flow_message_panicking_fn_returns_callback_take_msg() {
        fn panicking(_: InboundMessage) {
            panic!("intentional panic – must be caught");
        }
        let msg_p = alloc_msg();
        let (raw, _owner) = wrap_msg_fn(panicking);
        let rc = static_on_flow_message::<fn(InboundMessage)>(ptr::null_mut(), msg_p, raw);
        assert_eq!(
            rc,
            ffi::solClient_rxMsgCallback_returnCode_SOLCLIENT_CALLBACK_TAKE_MSG
        );
    }

    // --- static_on_event ---

    #[test]
    fn on_event_null_closure_does_not_crash() {
        let mut info = make_session_event_info();
        // null cookie → early return, no closure invoked
        static_on_event::<fn(SessionEvent)>(
            ptr::null_mut(),
            &mut info as *mut _,
            ptr::null_mut(),
        );
    }

    #[test]
    fn on_event_panicking_fn_does_not_abort() {
        fn panicking(_: SessionEvent) {
            panic!("intentional panic – must be caught");
        }
        let mut info = make_session_event_info();
        let (raw, _owner) = wrap_event_fn(panicking);
        static_on_event::<fn(SessionEvent)>(ptr::null_mut(), &mut info as *mut _, raw);
        // reaching here means catch_unwind prevented abort
    }

    // --- static_on_flow_event ---

    #[test]
    fn on_flow_event_null_closure_does_not_crash() {
        let mut info = make_flow_event_info();
        static_on_flow_event::<fn(FlowEvent)>(
            ptr::null_mut(),
            &mut info as *mut _,
            ptr::null_mut(),
        );
    }

    #[test]
    fn on_flow_event_panicking_fn_does_not_abort() {
        fn panicking(_: FlowEvent) {
            panic!("intentional panic – must be caught");
        }
        let mut info = make_flow_event_info();
        let (raw, _owner) = wrap_flow_event_fn(panicking);
        static_on_flow_event::<fn(FlowEvent)>(ptr::null_mut(), &mut info as *mut _, raw);
    }
}

pub(crate) fn get_last_error_info() -> SolClientSubCode {
    // Safety: erno is never null per the Solace C API contract.
    unsafe {
        let erno = solClient_getLastErrorInfo();
        let subcode = (*erno).subCode;
        let error_str_bytes = std::slice::from_raw_parts(
            (*erno).errorStr.as_ptr() as *const u8,
            (*erno).errorStr.len(),
        );
        let repr = std::ffi::CStr::from_bytes_until_nul(error_str_bytes).unwrap();
        SolClientSubCode {
            subcode,
            error_string: repr.to_string_lossy().to_string(),
        }
    }
}
