#[cfg(feature = "async")]
pub mod async_support;
pub mod cache_session;
pub mod context;
pub mod flow;
pub mod message;
pub mod session;
pub(crate) mod util;

use enum_primitive::*;
use solace_rs_sys as ffi;
use std::fmt::{self, Display};
use thiserror::Error;

pub use crate::context::Context;
pub use crate::message::outbound::MessageBuilderError;
pub use crate::message::MessageError;
pub use crate::session::builder::SessionBuilderError;
pub use crate::session::Session;

/// Top-level error type that wraps all sub-errors in this crate.
///
/// Use `?` to propagate any crate error into `SolaceError` in application code.
#[derive(Error, Debug)]
pub enum SolaceError {
    #[error(transparent)]
    Context(#[from] ContextError),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error(transparent)]
    SessionBuilder(#[from] SessionBuilderError),
    #[error(transparent)]
    Flow(#[from] FlowError),
    #[error(transparent)]
    Message(#[from] MessageError),
    #[error(transparent)]
    MessageBuilder(#[from] MessageBuilderError),
    #[cfg(feature = "async")]
    #[error(transparent)]
    AsyncSession(#[from] async_support::AsyncSessionError),
}

enum_from_primitive! {
    #[derive(Debug, PartialEq, Eq)]
    #[repr(u32)]
    pub enum SolaceLogLevel {
        Critical = ffi::solClient_log_level_SOLCLIENT_LOG_CRITICAL,
        Error = ffi::solClient_log_level_SOLCLIENT_LOG_ERROR,
        Warning = ffi::solClient_log_level_SOLCLIENT_LOG_WARNING,
        Notice = ffi::solClient_log_level_SOLCLIENT_LOG_NOTICE,
        Info = ffi::solClient_log_level_SOLCLIENT_LOG_INFO,
        Debug = ffi::solClient_log_level_SOLCLIENT_LOG_DEBUG,
    }
}

enum_from_primitive! {
    #[derive(PartialEq, Eq)]
    #[repr(i32)]
    pub enum SolClientReturnCode {
        Ok=ffi::solClient_returnCode_SOLCLIENT_OK,
        WouldBlock=ffi::solClient_returnCode_SOLCLIENT_WOULD_BLOCK,
        InProgress=ffi::solClient_returnCode_SOLCLIENT_IN_PROGRESS,
        NotReady=ffi::solClient_returnCode_SOLCLIENT_NOT_READY,
        EndOfStream=ffi::solClient_returnCode_SOLCLIENT_EOS,
        NotFound=ffi::solClient_returnCode_SOLCLIENT_NOT_FOUND,
        NoEvent=ffi::solClient_returnCode_SOLCLIENT_NOEVENT,
        Incomplete=ffi::solClient_returnCode_SOLCLIENT_INCOMPLETE,
        Rollback=ffi::solClient_returnCode_SOLCLIENT_ROLLBACK,
        Fail=ffi::solClient_returnCode_SOLCLIENT_FAIL,
    }
}

impl std::fmt::Display for SolClientReturnCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SolClientReturnCode::Ok => write!(f, "Ok - The API call was successful."),
            SolClientReturnCode::WouldBlock => write!(
                f,
                "WouldBlock - The API call would block, but non-blocking was requested."
            ),
            SolClientReturnCode::InProgress => write!(
                f,
                "InProgress - An API call is in progress (non-blocking mode)."
            ),
            SolClientReturnCode::NotReady => write!(f, "NotReady - The API could not complete as an object is not ready (for example, the Session is not connected)."),
            SolClientReturnCode::EndOfStream => write!(f, "EndOfStream - A getNext on a structured container returned End-of-Stream."),
            SolClientReturnCode::NotFound => write!(f, "NotFound - A get for a named field in a MAP was not found in the MAP."),
            SolClientReturnCode::NoEvent => write!(f, "NoEvent - solClient_context_processEventsWait returns this if wait is zero and there is no event to process"),
            SolClientReturnCode::Incomplete => write!(f, "Incomplete - The API call completed some, but not all, of the requested function."),
            SolClientReturnCode::Rollback => write!(f, "Rollback - solClient_transactedSession_commit returns this when the transaction has been rolled back."),
            SolClientReturnCode::Fail => write!(f, "Fail - The API call failed."),
        }
    }
}

impl std::fmt::Debug for SolClientReturnCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl SolClientReturnCode {
    pub fn from_raw(value: i32) -> Self {
        match Self::from_i32(value) {
            Some(rc) => rc,
            None => Self::Fail,
        }
    }

    pub fn is_ok(&self) -> bool {
        *self == Self::Ok
    }
}

#[derive(Debug)]
pub struct SolClientSubCode {
    pub subcode: u32,
    pub error_string: String,
}

impl Display for SolClientSubCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "subcode: {} string: {}", self.subcode, self.error_string)
    }
}

#[derive(Error, Debug)]
pub enum ContextError {
    #[error("context thread failed to initialize. SolClient return code: {0:?}")]
    InitializationFailed(SolClientReturnCode, SolClientSubCode),
}

#[derive(Error, Debug)]
pub enum FlowError {
    #[error("flow received arguments with null value")]
    InvalidArgsNulError(std::ffi::NulError),
    #[error("flow missing required argument: {0}")]
    MissingRequiredArgs(String),
    #[error("flow failed to create. SolClient return code: {0} subcode: {1}")]
    CreationFailure(SolClientReturnCode, SolClientSubCode),
    #[error("flow failed to start. SolClient return code: {0} subcode: {1}")]
    StartFailure(SolClientReturnCode, SolClientSubCode),
    #[error("flow failed to stop. SolClient return code: {0} subcode: {1}")]
    StopFailure(SolClientReturnCode, SolClientSubCode),
    #[error("flow failed to ack message id {0}. SolClient return code: {1} subcode: {2}")]
    AckFailure(u64, SolClientReturnCode, SolClientSubCode),
    #[error("flow failed to settle message id {0}. SolClient return code: {1} subcode: {2}")]
    SettleFailure(u64, SolClientReturnCode, SolClientSubCode),
}

#[derive(Error, Debug)]
pub enum SessionError {
    #[error("session received arguments with null value")]
    InvalidArgsNulError(#[from] std::ffi::NulError),
    #[error("session failed to connect. SolClient return code: {0} subcode: {1}")]
    ConnectionFailure(SolClientReturnCode, SolClientSubCode),
    #[error("session failed to disconnect. SolClient return code: {0} subcode: {1}")]
    DisconnectError(SolClientReturnCode, SolClientSubCode),
    #[error("session failed to initialize. SolClient return code: {0} subcode: {1}")]
    InitializationFailure(SolClientReturnCode, SolClientSubCode),
    #[error("session failed to subscribe on topic. SolClient return code: {0} subcode: {1}")]
    SubscriptionFailure(String, SolClientReturnCode, SolClientSubCode),
    #[error("session failed to unsubscribe on topic. SolClient return code: {0} subcode: {1}")]
    UnsubscriptionFailure(String, SolClientReturnCode, SolClientSubCode),
    #[error("cache request failed")]
    CacheRequestFailure(SolClientReturnCode, SolClientSubCode),
    #[error("could not publish message. SolClient return code: {0}")]
    PublishError(SolClientReturnCode, SolClientSubCode),
    #[error("could not send request. SolClient return code: {0}")]
    RequestError(SolClientReturnCode, SolClientSubCode),
    #[error("broker rejected guaranteed message")]
    AcknowledgementRejected,
    #[error("cannot disconnect: drop all OwnedAsyncFlow instances first")]
    ActiveFlowsOnDisconnect,
}
