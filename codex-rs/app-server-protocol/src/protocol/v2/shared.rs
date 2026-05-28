use codex_protocol::protocol::CodexErrorInfo as CoreCodexErrorInfo;
use codex_protocol::protocol::NonSteerableTurnKind as CoreNonSteerableTurnKind;
use serde::Deserialize;
use serde::Serialize;

macro_rules! v2_enum_from_core {
    (
        $(#[$enum_meta:meta])*
        pub enum $Name:ident from $Src:path {
            $( $(#[$variant_meta:meta])* $Variant:ident ),+ $(,)?
        }
    ) => {
        #[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
        $(#[$enum_meta])*
        #[serde(rename_all = "camelCase")]
        pub enum $Name {
            $( $(#[$variant_meta])* $Variant ),+
        }

        impl $Name {
            pub fn to_core(self) -> $Src {
                match self {
                    $( $Name::$Variant => <$Src>::$Variant ),+
                }
            }
        }

        impl From<$Src> for $Name {
            fn from(value: $Src) -> Self {
                match value {
                    $( <$Src>::$Variant => $Name::$Variant ),+
                }
            }
        }
    };
}

pub(crate) use v2_enum_from_core;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum NonSteerableTurnKind {
    Compact,
}

impl From<CoreNonSteerableTurnKind> for NonSteerableTurnKind {
    fn from(value: CoreNonSteerableTurnKind) -> Self {
        match value {
            CoreNonSteerableTurnKind::Compact => Self::Compact,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CodexErrorInfo {
    ContextWindowExceeded,
    UsageLimitExceeded,
    ServerOverloaded,
    CyberPolicy,
    HttpConnectionFailed {
        #[serde(rename = "httpStatusCode")]
        http_status_code: Option<u16>,
    },
    ResponseStreamConnectionFailed {
        #[serde(rename = "httpStatusCode")]
        http_status_code: Option<u16>,
    },
    InternalServerError,
    Unauthorized,
    BadRequest,
    ResponseStreamDisconnected {
        #[serde(rename = "httpStatusCode")]
        http_status_code: Option<u16>,
    },
    ResponseTooManyFailedAttempts {
        #[serde(rename = "httpStatusCode")]
        http_status_code: Option<u16>,
    },
    ActiveTurnNotSteerable {
        #[serde(rename = "turnKind")]
        turn_kind: NonSteerableTurnKind,
    },
    Other,
}

impl From<CoreCodexErrorInfo> for CodexErrorInfo {
    fn from(value: CoreCodexErrorInfo) -> Self {
        match value {
            CoreCodexErrorInfo::ContextWindowExceeded => CodexErrorInfo::ContextWindowExceeded,
            CoreCodexErrorInfo::UsageLimitExceeded => CodexErrorInfo::UsageLimitExceeded,
            CoreCodexErrorInfo::ServerOverloaded => CodexErrorInfo::ServerOverloaded,
            CoreCodexErrorInfo::CyberPolicy => CodexErrorInfo::CyberPolicy,
            CoreCodexErrorInfo::HttpConnectionFailed { http_status_code } => {
                CodexErrorInfo::HttpConnectionFailed { http_status_code }
            }
            CoreCodexErrorInfo::ResponseStreamConnectionFailed { http_status_code } => {
                CodexErrorInfo::ResponseStreamConnectionFailed { http_status_code }
            }
            CoreCodexErrorInfo::InternalServerError => CodexErrorInfo::InternalServerError,
            CoreCodexErrorInfo::Unauthorized => CodexErrorInfo::Unauthorized,
            CoreCodexErrorInfo::BadRequest => CodexErrorInfo::BadRequest,
            CoreCodexErrorInfo::ResponseStreamDisconnected { http_status_code } => {
                CodexErrorInfo::ResponseStreamDisconnected { http_status_code }
            }
            CoreCodexErrorInfo::ResponseTooManyFailedAttempts { http_status_code } => {
                CodexErrorInfo::ResponseTooManyFailedAttempts { http_status_code }
            }
            CoreCodexErrorInfo::ActiveTurnNotSteerable { turn_kind } => {
                CodexErrorInfo::ActiveTurnNotSteerable {
                    turn_kind: turn_kind.into(),
                }
            }
            CoreCodexErrorInfo::Other => CodexErrorInfo::Other,
        }
    }
}
