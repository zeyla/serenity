use std::{
    error::Error as StdError,
    fmt::{self, Display, Error as FormatError},
    io::Error as IoError,
    num::ParseIntError,
};

#[cfg(feature = "gateway")]
use async_tungstenite::tungstenite::error::Error as TungsteniteError;
#[cfg(feature = "http")]
use reqwest::{header::InvalidHeaderValue, Error as ReqwestError};
use serde_json::Error as JsonError;
use tracing::instrument;

#[cfg(feature = "client")]
use crate::client::ClientError;
#[cfg(feature = "collector")]
use crate::collector::CollectorError;
#[cfg(feature = "gateway")]
use crate::gateway::GatewayError;
#[cfg(feature = "http")]
use crate::http::HttpError;
use crate::internal::prelude::*;
#[cfg(all(
    feature = "gateway",
    feature = "rustls_backend_marker",
    not(feature = "native_tls_backend_marker")
))]
use crate::internal::ws_impl::RustlsError;
use crate::model::ModelError;

/// The common result type between most library functions.
///
/// The library exposes functions which, for a result type, exposes only one
/// type, rather than the usual 2 (`Result<T, Error>`). This is because all
/// functions that return a result return serenity's [`Error`], so this is
/// implied, and a "simpler" result is used.
pub type Result<T> = StdResult<T, Error>;

/// A common error enum returned by most of the library's functionality within a
/// custom [`Result`].
///
/// The most common error types, the [`ClientError`] and [`GatewayError`]
/// enums, are both wrapped around this in the form of the [`Self::Client`] and
/// [`Self::Gateway`] variants.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// An error while decoding a payload.
    Decode(&'static str, Value),
    /// There was an error with a format.
    Format(FormatError),
    /// An [`std::io`] error.
    Io(IoError),
    /// An error from the [`serde_json`] crate.
    Json(JsonError),
    /// An error from the [`model`] module.
    ///
    /// [`model`]: crate::model
    Model(ModelError),
    /// An error occurred while parsing an integer.
    Num(ParseIntError),
    /// Input exceeded a limit.
    /// Providing the input and the limit that's not supposed to be exceeded.
    ///
    /// *This only exists for the [`GuildId::ban`] and [`Member::ban`] functions. For their cases,
    /// it's the "reason".*
    ///
    /// [`GuildId::ban`]: crate::model::id::GuildId::ban
    /// [`Member::ban`]: crate::model::guild::Member::ban
    ExceededLimit(String, u32),
    /// The input is not in the specified range.
    /// Returned by [`GuildId::members`], [`Guild::members`] and [`PartialGuild::members`]
    ///
    /// (param_name, value, range_min, range_max)
    ///
    /// [`GuildId::members`]: crate::model::id::GuildId::members
    /// [`Guild::members`]: crate::model::guild::Guild::members
    /// [`PartialGuild::members`]: crate::model::guild::PartialGuild::members
    NotInRange(&'static str, u64, u64, u64),
    /// Some other error. This is only used for "Expected value <TYPE>" errors,
    /// when a more detailed error can not be easily provided via the
    /// [`Error::Decode`] variant.
    Other(&'static str),
    /// An error from the [`url`] crate.
    Url(String),
    /// A [client] error.
    ///
    /// [client]: crate::client
    #[cfg(feature = "client")]
    Client(ClientError),
    /// A [collector] error.
    ///
    /// [collector]: crate::collector
    #[cfg(feature = "collector")]
    Collector(CollectorError),
    /// An error from the [`gateway`] module.
    ///
    /// [`gateway`]: crate::gateway
    #[cfg(feature = "gateway")]
    Gateway(GatewayError),
    /// An error from the [`http`] module.
    ///
    /// [`http`]: crate::http
    #[cfg(feature = "http")]
    Http(Box<HttpError>),
    /// An error occuring in rustls
    #[cfg(all(
        feature = "gateway",
        feature = "rustls_backend_marker",
        not(feature = "native_tls_backend_marker")
    ))]
    Rustls(RustlsError),
    /// An error from the `tungstenite` crate.
    #[cfg(feature = "gateway")]
    Tungstenite(TungsteniteError),
}

impl From<FormatError> for Error {
    fn from(e: FormatError) -> Error {
        Error::Format(e)
    }
}

#[cfg(feature = "gateway")]
impl From<GatewayError> for Error {
    fn from(e: GatewayError) -> Error {
        Error::Gateway(e)
    }
}

impl From<IoError> for Error {
    fn from(e: IoError) -> Error {
        Error::Io(e)
    }
}

impl From<JsonError> for Error {
    fn from(e: JsonError) -> Error {
        Error::Json(e)
    }
}

impl From<ParseIntError> for Error {
    fn from(e: ParseIntError) -> Error {
        Error::Num(e)
    }
}

impl From<ModelError> for Error {
    fn from(e: ModelError) -> Error {
        Error::Model(e)
    }
}

#[cfg(all(
    feature = "gateway",
    feature = "rustls_backend_marker",
    not(feature = "native_tls_backend_marker")
))]
impl From<RustlsError> for Error {
    fn from(e: RustlsError) -> Error {
        Error::Rustls(e)
    }
}

#[cfg(feature = "gateway")]
impl From<TungsteniteError> for Error {
    fn from(e: TungsteniteError) -> Error {
        Error::Tungstenite(e)
    }
}

#[cfg(feature = "http")]
impl From<HttpError> for Error {
    fn from(e: HttpError) -> Error {
        Error::Http(Box::new(e))
    }
}

#[cfg(feature = "http")]
impl From<InvalidHeaderValue> for Error {
    fn from(e: InvalidHeaderValue) -> Error {
        HttpError::InvalidHeader(e).into()
    }
}

#[cfg(feature = "http")]
impl From<ReqwestError> for Error {
    fn from(e: ReqwestError) -> Error {
        HttpError::Request(e).into()
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Decode(msg, _) | Error::Other(msg) => f.write_str(msg),
            Error::ExceededLimit(..) => f.write_str("Input exceeded a limit"),
            Error::NotInRange(..) => f.write_str("Input is not in the specified range"),
            Error::Format(inner) => fmt::Display::fmt(&inner, f),
            Error::Io(inner) => fmt::Display::fmt(&inner, f),
            Error::Json(inner) => fmt::Display::fmt(&inner, f),
            Error::Model(inner) => fmt::Display::fmt(&inner, f),
            Error::Num(inner) => fmt::Display::fmt(&inner, f),
            Error::Url(msg) => f.write_str(msg),
            #[cfg(feature = "client")]
            Error::Client(inner) => fmt::Display::fmt(&inner, f),
            #[cfg(feature = "collector")]
            Error::Collector(inner) => fmt::Display::fmt(&inner, f),
            #[cfg(feature = "gateway")]
            Error::Gateway(inner) => fmt::Display::fmt(&inner, f),
            #[cfg(feature = "http")]
            Error::Http(inner) => fmt::Display::fmt(&inner, f),
            #[cfg(all(feature = "gateway", not(feature = "native_tls_backend_marker")))]
            Error::Rustls(inner) => fmt::Display::fmt(&inner, f),
            #[cfg(feature = "gateway")]
            Error::Tungstenite(inner) => fmt::Display::fmt(&inner, f),
        }
    }
}

impl StdError for Error {
    #[instrument]
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Error::Format(inner) => Some(inner),
            Error::Io(inner) => Some(inner),
            Error::Json(inner) => Some(inner),
            Error::Model(inner) => Some(inner),
            Error::Num(inner) => Some(inner),
            #[cfg(feature = "client")]
            Error::Client(inner) => Some(inner),
            #[cfg(feature = "collector")]
            Error::Collector(inner) => Some(inner),
            #[cfg(feature = "gateway")]
            Error::Gateway(inner) => Some(inner),
            #[cfg(feature = "http")]
            Error::Http(inner) => Some(inner),
            #[cfg(all(
                feature = "gateway",
                feature = "rustls_backend_marker",
                not(feature = "native_tls_backend_marker")
            ))]
            Error::Rustls(inner) => Some(inner),
            #[cfg(feature = "gateway")]
            Error::Tungstenite(inner) => Some(inner),
            _ => None,
        }
    }
}
