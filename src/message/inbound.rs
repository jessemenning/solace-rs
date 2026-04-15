use super::{CacheStatus, Message, MessageError, Result};
use crate::SolClientReturnCode;
use enum_primitive::*;
use solace_rs_sys as ffi;
use std::collections::HashMap;
use std::convert::From;
use std::ffi::CStr;
use std::time::{Duration, SystemTime};
use std::{fmt, mem, ptr};
use tracing::warn;

pub struct InboundMessage {
    _msg_ptr: ffi::solClient_opaqueMsg_pt,
}

impl fmt::Debug for InboundMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut f = f.debug_struct("InboundMessage");
        if self.get_receive_timestamp().is_ok_and(|v| v.is_some()) {
            f.field(
                "receive_timestamp",
                &format_args!("{:?}", self.get_receive_timestamp().unwrap().unwrap()),
            );
        }
        if self.get_sender_id().is_ok_and(|v| v.is_some()) {
            f.field(
                "sender_id",
                &format_args!("{}", self.get_sender_id().unwrap().unwrap()),
            );
        }
        if self.get_sender_timestamp().is_ok_and(|v| v.is_some()) {
            f.field(
                "sender_timestamp",
                &format_args!("{:?}", self.get_sender_timestamp().unwrap().unwrap()),
            );
        }
        if self.get_sequence_number().is_ok_and(|v| v.is_some()) {
            f.field(
                "sequence_number",
                &format_args!("{}", self.get_sequence_number().unwrap().unwrap()),
            );
        }
        if self.get_correlation_id().is_ok_and(|v| v.is_some()) {
            f.field(
                "correlation_id",
                &format_args!("{}", self.get_correlation_id().unwrap().unwrap()),
            );
        }
        if self.get_priority().is_ok_and(|v| v.is_some()) {
            f.field(
                "priority",
                &format_args!("{}", self.get_priority().unwrap().unwrap()),
            );
        }
        if self.is_discard_indication() {
            f.field(
                "is_discard_indication",
                &format_args!("{}", self.is_discard_indication()),
            );
        }
        if self.get_application_message_id().is_some() {
            f.field(
                "application_message_id",
                &format_args!("{}", &self.get_application_message_id().unwrap()),
            );
        }
        if self.get_user_data().is_ok_and(|v| v.is_some()) {
            if let Ok(v) = std::str::from_utf8(self.get_user_data().unwrap().unwrap()) {
                f.field("user_data", &v);
            }
        }
        if self.get_destination().is_ok_and(|v| v.is_some()) {
            f.field("destination", &self.get_destination().unwrap().unwrap());
        }

        f.field("is_reply", &self.is_reply());

        if self.get_reply_to().is_ok_and(|v| v.is_some()) {
            f.field("reply_to", &self.get_reply_to().unwrap().unwrap());
        }

        f.field("is_cache_msg", &self.is_cache_msg());

        if self.get_cache_request_id().is_ok_and(|v| v.is_some()) {
            f.field(
                "cache_request_id",
                &self.get_cache_request_id().unwrap().unwrap(),
            );
        }

        if self.get_payload().is_ok_and(|v| v.is_some()) {
            if let Ok(v) = std::str::from_utf8(self.get_payload().unwrap().unwrap()) {
                f.field("payload", &v);
            }
        }
        f.finish()
    }
}

unsafe impl Send for InboundMessage {}

impl Drop for InboundMessage {
    fn drop(&mut self) {
        let rc = unsafe { ffi::solClient_msg_free(&mut self._msg_ptr) };

        let rc = SolClientReturnCode::from_raw(rc);
        if !rc.is_ok() {
            warn!("warning: message was not dropped properly");
        }
    }
}

impl From<ffi::solClient_opaqueMsg_pt> for InboundMessage {
    /// .
    ///
    /// # Safety
    ///
    /// From a valid owned pointer.
    /// No other alias should exist for this pointer
    /// InboundMessage will try to free the ptr when it is destroyed
    ///
    /// .
    fn from(ptr: ffi::solClient_opaqueMsg_pt) -> Self {
        Self { _msg_ptr: ptr }
    }
}

impl Message for InboundMessage {
    unsafe fn get_raw_message_ptr(&self) -> ffi::solClient_opaqueMsg_pt {
        self._msg_ptr
    }
}

impl InboundMessage {
    pub fn get_receive_timestamp(&self) -> Result<Option<SystemTime>> {
        let mut ts: i64 = 0;
        let rc = unsafe { ffi::solClient_msg_getRcvTimestamp(self.get_raw_message_ptr(), &mut ts) };

        let rc = SolClientReturnCode::from_raw(rc);
        match rc {
            SolClientReturnCode::NotFound => Ok(None),
            SolClientReturnCode::Ok => Ok(Some(
                SystemTime::UNIX_EPOCH + Duration::from_millis(ts.try_into().unwrap()),
            )),
            _ => Err(MessageError::FieldError("receive_timestamp", rc)),
        }
    }

    pub fn get_sender_id(&self) -> Result<Option<&str>> {
        let mut buffer = ptr::null();

        let rc = unsafe { ffi::solClient_msg_getSenderId(self.get_raw_message_ptr(), &mut buffer) };

        let rc = SolClientReturnCode::from_raw(rc);
        match rc {
            SolClientReturnCode::Ok => (),
            SolClientReturnCode::NotFound => return Ok(None),
            _ => return Err(MessageError::FieldError("sender_id", rc)),
        }

        let c_str = unsafe { CStr::from_ptr(buffer) };

        let str = c_str
            .to_str()
            .map_err(|_| MessageError::FieldConvertionError("sender_id"))?;

        Ok(Some(str))
    }

    pub fn is_discard_indication(&self) -> bool {
        let discard_indication =
            unsafe { ffi::solClient_msg_isDiscardIndication(self.get_raw_message_ptr()) };

        if discard_indication == 0 {
            return false;
        }

        true
    }

    pub fn get_cache_request_id(&self) -> Result<Option<u64>> {
        let mut id: u64 = 0;

        let rc =
            unsafe { ffi::solClient_msg_getCacheRequestId(self.get_raw_message_ptr(), &mut id) };

        let rc = SolClientReturnCode::from_raw(rc);
        match rc {
            SolClientReturnCode::Ok => Ok(Some(id)),
            SolClientReturnCode::NotFound => Ok(None),
            _ => Err(MessageError::FieldError("cache_request_id", rc)),
        }
    }

    pub fn is_cache_msg(&self) -> CacheStatus {
        let raw = unsafe { ffi::solClient_msg_isCacheMsg(self.get_raw_message_ptr()) };
        CacheStatus::from_i32(raw).unwrap_or(CacheStatus::InvalidMessage)
    }

    /// Get user properties as a string key-value map.
    ///
    /// Returns an empty map if no user properties are set on the message.
    /// Only string-valued properties are extracted; other types are skipped.
    pub fn get_user_properties(&self) -> Result<HashMap<String, String>> {
        let mut map_p: ffi::solClient_opaqueContainer_pt = ptr::null_mut();
        let rc = unsafe {
            ffi::solClient_msg_getUserPropertyMap(self.get_raw_message_ptr(), &mut map_p)
        };

        let rc = SolClientReturnCode::from_raw(rc);
        match rc {
            SolClientReturnCode::Ok => {}
            SolClientReturnCode::NotFound => return Ok(HashMap::new()),
            _ => return Err(MessageError::FieldError("user_properties", rc)),
        }

        let mut props = HashMap::new();
        loop {
            let mut field: ffi::solClient_field_t = unsafe { mem::zeroed() };
            let mut name_p: *const std::os::raw::c_char = ptr::null();

            let rc = unsafe {
                ffi::solClient_container_getNextField(
                    map_p,
                    &mut field,
                    mem::size_of::<ffi::solClient_field_t>(),
                    &mut name_p,
                )
            };

            let rc = SolClientReturnCode::from_raw(rc);
            if rc == SolClientReturnCode::EndOfStream || !rc.is_ok() {
                break;
            }

            if name_p.is_null() {
                continue;
            }

            let name = unsafe { CStr::from_ptr(name_p) };
            let name = match name.to_str() {
                Ok(s) => s.to_string(),
                Err(_) => continue,
            };

            // Only extract string-type fields
            if field.type_ == ffi::solClient_fieldType_SOLCLIENT_STRING {
                let value_ptr = unsafe { field.value.string };
                if !value_ptr.is_null() {
                    let value = unsafe { CStr::from_ptr(value_ptr) };
                    if let Ok(v) = value.to_str() {
                        props.insert(name, v.to_string());
                    }
                }
            }
        }

        Ok(props)
    }

    /// Get the message ID for use with flow ACK/settle operations.
    ///
    /// Returns `None` for direct messages (which have no message ID).
    pub fn get_msg_id(&self) -> Result<Option<u64>> {
        let mut msg_id: ffi::solClient_msgId_t = 0;
        let rc = unsafe { ffi::solClient_msg_getMsgId(self.get_raw_message_ptr(), &mut msg_id) };

        let rc = SolClientReturnCode::from_raw(rc);
        match rc {
            SolClientReturnCode::Ok => Ok(Some(msg_id)),
            SolClientReturnCode::NotFound => Ok(None),
            _ => Err(MessageError::FieldError("msg_id", rc)),
        }
    }

    /// Returns `true` if the broker has redelivered this message after a previous
    /// delivery attempt was not acknowledged.
    pub fn is_redelivered(&self) -> bool {
        let rc = unsafe { ffi::solClient_msg_isRedelivered(self.get_raw_message_ptr()) };
        rc != 0
    }

    /// Retrieves the Replication Group Message ID as a string.
    ///
    /// Returns `None` if the message does not carry a replication group message ID
    /// (e.g. direct messages).
    pub fn get_replication_group_message_id(&self) -> Result<Option<String>> {
        let mut rgmid = ffi::solClient_replicationGroupMessageId_t {
            replicationGroupMessageId: [0; 16],
        };
        let rc = unsafe {
            ffi::solClient_msg_getReplicationGroupMessageId(
                self.get_raw_message_ptr(),
                &mut rgmid,
                std::mem::size_of::<ffi::solClient_replicationGroupMessageId_t>(),
            )
        };
        let rc = SolClientReturnCode::from_raw(rc);
        match rc {
            SolClientReturnCode::NotFound => return Ok(None),
            SolClientReturnCode::Ok => {}
            _ => {
                return Err(MessageError::FieldError(
                    "replication_group_message_id",
                    rc,
                ))
            }
        }

        // Convert opaque struct to a 41-char string
        // (SOLCLIENT_REPLICATION_GROUP_MESSAGE_ID_STRING_LENGTH = 41)
        let mut buf = [0i8; 41];
        let rc = unsafe {
            ffi::solClient_replicationGroupMessageId_toString(
                &mut rgmid,
                std::mem::size_of::<ffi::solClient_replicationGroupMessageId_t>(),
                buf.as_mut_ptr(),
                buf.len(),
            )
        };
        let rc = SolClientReturnCode::from_raw(rc);
        if rc != SolClientReturnCode::Ok {
            return Err(MessageError::FieldError(
                "replication_group_message_id_to_string",
                rc,
            ));
        }

        let s = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }
            .to_str()
            .map_err(|_| MessageError::FieldConvertionError("replication_group_message_id"))?
            .to_owned();
        Ok(Some(s))
    }
}
