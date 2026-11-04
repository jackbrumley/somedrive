use super::*;

// Why a sync cycle stopped, decided where the failure happened (an HTTP
// status, a network error, an I/O error kind), never guessed from the text.

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SyncError {
    /// Pause, quit or update: not a failure.
    Cancelled,
    /// The sync folder is missing or empty while files are tracked there.
    SyncRootUnavailable(String),
    /// OneDrive kept answering 429 after the retries.
    RateLimited(String),
    /// No connection, a timeout, or OneDrive unavailable (5xx).
    Network(String),
    /// OneDrive (403) or the local file system refused access.
    PermissionDenied(String),
    /// The local disk is full.
    DiskFull(String),
    /// Anything else (database, decoding, unexpected responses).
    Other(String),
}

pub(crate) type SyncResult<T> = Result<T, SyncError>;

impl SyncError {
    pub(crate) fn from_status(status: StatusCode, message: String) -> Self {
        if status == StatusCode::TOO_MANY_REQUESTS {
            Self::RateLimited(message)
        } else if status == StatusCode::FORBIDDEN {
            Self::PermissionDenied(message)
        } else if status.is_server_error() {
            Self::Network(message)
        } else {
            Self::Other(message)
        }
    }

    pub(crate) fn from_request(error: &reqwest::Error, message: String) -> Self {
        if error.is_connect() || error.is_timeout() || error.is_request() {
            Self::Network(message)
        } else {
            Self::Other(message)
        }
    }

    pub(crate) fn from_io(error: &std::io::Error, message: String) -> Self {
        match error.kind() {
            std::io::ErrorKind::PermissionDenied => Self::PermissionDenied(message),
            std::io::ErrorKind::StorageFull => Self::DiskFull(message),
            _ => Self::Other(message),
        }
    }

    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Cancelled => SYNC_CANCELLED_ERROR,
            Self::SyncRootUnavailable(message)
            | Self::RateLimited(message)
            | Self::Network(message)
            | Self::PermissionDenied(message)
            | Self::DiskFull(message)
            | Self::Other(message) => message,
        }
    }

    /// The issue code and actions the UI shows for this failure.
    pub(crate) fn issue(&self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::Cancelled => ("cancelled", &[]),
            Self::SyncRootUnavailable(_) => {
                ("sync_root_unavailable", &["open_sync_root", "retry_sync"])
            }
            Self::RateLimited(_) => ("rate_limited", &["retry_sync"]),
            Self::Network(_) => ("network_unavailable", &["retry_sync"]),
            Self::PermissionDenied(_) => ("permission_denied", &["open_sync_root", "retry_sync"]),
            Self::DiskFull(_) => ("disk_full", &["open_sync_root", "retry_sync"]),
            Self::Other(_) => ("unknown_error", &["retry_sync"]),
        }
    }
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

/// Helpers that report plain text (the database store, decoding) are
/// `Other`; the cancellation sentinel stays a cancellation.
impl From<String> for SyncError {
    fn from(message: String) -> Self {
        if message == SYNC_CANCELLED_ERROR {
            Self::Cancelled
        } else {
            Self::Other(message)
        }
    }
}

#[cfg(test)]
mod sync_error_tests {
    use super::*;

    #[test]
    fn failures_map_to_issues_from_their_kind_not_their_text() {
        assert_eq!(
            SyncError::from_status(StatusCode::TOO_MANY_REQUESTS, "x".into())
                .issue()
                .0,
            "rate_limited"
        );
        assert_eq!(
            SyncError::from_status(StatusCode::FORBIDDEN, "x".into())
                .issue()
                .0,
            "permission_denied"
        );
        assert_eq!(
            SyncError::from_status(StatusCode::BAD_GATEWAY, "x".into())
                .issue()
                .0,
            "network_unavailable"
        );
        assert_eq!(
            SyncError::from_io(
                &std::io::Error::from(std::io::ErrorKind::StorageFull),
                "x".into()
            )
            .issue()
            .0,
            "disk_full"
        );
        // Text that merely mentions a network does not make it a network error.
        assert_eq!(
            SyncError::from("network share 403".to_string()).issue().0,
            "unknown_error"
        );
        assert_eq!(
            SyncError::from(SYNC_CANCELLED_ERROR.to_string()),
            SyncError::Cancelled
        );
    }
}
