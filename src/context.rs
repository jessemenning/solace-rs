use crate::session::builder::SessionBuilder;
use crate::session::builder::SessionBuilderError;
use crate::util::get_last_error_info;
use crate::Session;
use crate::{ContextError, SolClientReturnCode, SolaceLogLevel};
use solace_rs_sys as ffi;
use std::ffi::CString;
use std::mem;
use std::ptr;
use std::sync::Mutex;
use std::sync::OnceLock;
use tracing::warn;

use crate::message::InboundMessage;
use crate::session::SessionEvent;
use std::sync::Arc;
type Result<T> = std::result::Result<T, ContextError>;

pub(super) struct RawContext {
    // This pointer must never be allowed to leave the struct
    pub(crate) ctx: ffi::solClient_opaqueContext_pt,
}

static SOLACE_GLOBAL_INIT: OnceLock<i32> = OnceLock::new();

/// Global crypto-library configuration applied once, process-wide, at the first
/// `solClient_initialize`.
///
/// The Solace C API does not embed OpenSSL — on Linux it `dlopen`s `libssl` /
/// `libcrypto` by name at runtime (default search list `libssl.so.3`,
/// `libssl.so.1.1`, `libssl.so`, and the crypto equivalents). These two fields
/// are passed verbatim as the global properties `GLOBAL_SSL_LIB` /
/// `GLOBAL_CRYPTO_LIB`, letting you pin the exact library the API loads — for
/// example a FIPS-validated OpenSSL build whose `fips` provider is active.
///
/// Leave a field `None` to use the API's default search. Values may be a bare
/// SONAME (`libcrypto.so.3`) resolved via the loader path, or an absolute path
/// to a specific library file.
#[derive(Clone, Debug, Default)]
pub struct GlobalCryptoConfig {
    /// TLS/SSL library name or path (`GLOBAL_SSL_LIB`).
    pub ssl_lib: Option<Vec<u8>>,
    /// Cryptography library name or path (`GLOBAL_CRYPTO_LIB`).
    pub crypto_lib: Option<Vec<u8>>,
}

impl GlobalCryptoConfig {
    fn is_empty(&self) -> bool {
        self.ssl_lib.is_none() && self.crypto_lib.is_none()
    }
}

impl RawContext {
    /// .
    /// Raw solace context that wraps around the c context
    ///
    /// # Errors
    ///
    /// This function will return an error if .
    ///
    /// # Safety
    /// Context initializes global variables so it is not safe to have multiple solace contexts.
    /// .
    pub unsafe fn new(log_level: SolaceLogLevel, crypto: &GlobalCryptoConfig) -> Result<Self> {
        // The Solace C API loads OpenSSL at runtime; `GLOBAL_SSL_LIB` /
        // `GLOBAL_CRYPTO_LIB` pin which library it loads. These are global
        // properties consumed by `solClient_initialize`. The CStrings below must
        // outlive the call; they live for this function's scope, and
        // `get_or_init` runs its closure synchronously before returning.
        let ssl_lib = match &crypto.ssl_lib {
            Some(v) => Some(
                CString::new(v.clone())
                    .map_err(|e| ContextError::InvalidGlobalProperty("ssl_lib".to_owned(), e))?,
            ),
            None => None,
        };
        let crypto_lib = match &crypto.crypto_lib {
            Some(v) => Some(
                CString::new(v.clone())
                    .map_err(|e| ContextError::InvalidGlobalProperty("crypto_lib".to_owned(), e))?,
            ),
            None => None,
        };

        let mut global_props: Vec<*const std::os::raw::c_char> = Vec::new();
        if let Some(cs) = &ssl_lib {
            global_props
                .push(ffi::SOLCLIENT_GLOBAL_PROP_SSL_LIB.as_ptr() as *const std::os::raw::c_char);
            global_props.push(cs.as_ptr());
        }
        if let Some(cs) = &crypto_lib {
            global_props
                .push(ffi::SOLCLIENT_GLOBAL_PROP_CRYPTO_LIB.as_ptr() as *const std::os::raw::c_char);
            global_props.push(cs.as_ptr());
        }
        let props_ptr = if global_props.is_empty() {
            ptr::null_mut()
        } else {
            global_props.push(ptr::null());
            global_props.as_mut_ptr() as *mut *mut std::os::raw::c_char
        };

        let mut applied = false;
        let rc = SOLACE_GLOBAL_INIT.get_or_init(|| {
            applied = true;
            ffi::solClient_initialize(log_level as u32, props_ptr)
        });
        if !applied && !crypto.is_empty() {
            warn!(
                "GlobalCryptoConfig ignored: the Solace C API was already initialized by an \
                 earlier context in this process; GLOBAL_SSL_LIB/GLOBAL_CRYPTO_LIB take effect \
                 only on the first context created"
            );
        }

        let rc = SolClientReturnCode::from_raw(*rc);

        if !rc.is_ok() {
            let subcode = get_last_error_info();
            return Err(ContextError::InitializationFailed(rc, subcode));
        }
        let mut ctx: ffi::solClient_opaqueContext_pt = ptr::null_mut();
        let mut context_func: ffi::solClient_context_createFuncInfo_t =
            ffi::solClient_context_createFuncInfo {
                regFdInfo: ffi::solClient_context_createRegisterFdFuncInfo {
                    regFdFunc_p: None,
                    unregFdFunc_p: None,
                    user_p: ptr::null_mut(),
                },
            };

        // enable context thread
        let mut context_props: [*const std::os::raw::c_char; 3] = [
            solace_rs_sys::SOLCLIENT_CONTEXT_PROP_CREATE_THREAD.as_ptr()
                as *const std::os::raw::c_char,
            solace_rs_sys::SOLCLIENT_PROP_ENABLE_VAL.as_ptr() as *const std::os::raw::c_char,
            ptr::null(),
        ];

        let solace_context_raw_rc = unsafe {
            ffi::solClient_context_create(
                context_props.as_mut_ptr() as *mut *mut _,
                &mut ctx,
                &mut context_func,
                mem::size_of::<ffi::solClient_context_createRegisterFdFuncInfo>(),
            )
        };

        let rc = SolClientReturnCode::from_raw(solace_context_raw_rc);

        if !rc.is_ok() {
            let subcode = get_last_error_info();
            return Err(ContextError::InitializationFailed(rc, subcode));
        }
        Ok(Self { ctx })
    }
}

impl Drop for RawContext {
    fn drop(&mut self) {
        let return_code = unsafe { ffi::solClient_context_destroy(&mut self.ctx) };
        if return_code != ffi::solClient_returnCode_SOLCLIENT_OK {
            warn!("Solace context did not drop properly");
        }
    }
}

unsafe impl Send for RawContext {}

/// Handle for a Solace context, used to create sessions.
///
/// It is thread safe, and can be safely cloned and shared. Each clone
/// references the same underlying C context. Internally, an `Arc` is
/// used to implement this in a threadsafe way.
///
/// Also note that this binding deviates from the C API in that each
/// session created from a context initially owns a clone of that
/// context.
///
///
#[derive(Clone)]
pub struct Context {
    pub(super) raw: Arc<Mutex<RawContext>>,
}

impl Context {
    pub fn new(log_level: SolaceLogLevel) -> std::result::Result<Self, ContextError> {
        Self::with_crypto(log_level, &GlobalCryptoConfig::default())
    }

    /// Create a context, pinning the global OpenSSL/crypto libraries the Solace
    /// C API loads at runtime. See [`GlobalCryptoConfig`].
    ///
    /// Note: Solace performs global initialization once per process, so the
    /// crypto configuration takes effect only on the **first** context created
    /// in the process; later contexts with a different config are ignored (a
    /// warning is logged).
    pub fn with_crypto(
        log_level: SolaceLogLevel,
        crypto: &GlobalCryptoConfig,
    ) -> std::result::Result<Self, ContextError> {
        Ok(Self {
            raw: Arc::new(Mutex::new(unsafe { RawContext::new(log_level, crypto) }?)),
        })
    }

    pub fn session_builder<Host, Vpn, Username, Password, OnMessage, OnEvent>(
        &self,
    ) -> SessionBuilder<Host, Vpn, Username, Password, OnMessage, OnEvent> {
        SessionBuilder::new(self.clone())
    }

    pub fn session<'session, Host, Vpn, Username, Password, OnMessage, OnEvent>(
        &self,
        host_name: Host,
        vpn_name: Vpn,
        username: Username,
        password: Password,
        on_message: Option<OnMessage>,
        on_event: Option<OnEvent>,
    ) -> std::result::Result<Session<'session, OnMessage, OnEvent>, SessionBuilderError>
    where
        Host: Into<Vec<u8>>,
        Vpn: Into<Vec<u8>>,
        Username: Into<Vec<u8>>,
        Password: Into<Vec<u8>>,
        OnMessage: FnMut(InboundMessage) + Send + 'session,
        OnEvent: FnMut(SessionEvent) + Send + 'session,
    {
        let mut builder = SessionBuilder::new(self.clone())
            .host_name(host_name)
            .vpn_name(vpn_name)
            .username(username)
            .password(password);

        if let Some(on_message) = on_message {
            builder = builder.on_message(on_message);
        }

        if let Some(on_event) = on_event {
            builder = builder.on_event(on_event);
        }

        builder.build()
    }
}
