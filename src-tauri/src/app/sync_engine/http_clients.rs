use super::*;

/// Client for Graph API requests (metadata, the change feed, upload
/// sessions). These are small, so HTTP/2 multiplexes them over one
/// connection; the adaptive window keeps a large change-feed page from
/// stalling on the default 64 KB flow-control window.
pub(crate) fn graph_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(resolve_connect_timeout())
        .timeout(resolve_request_timeout())
        .http2_adaptive_window(true)
        .build()
        .map_err(|error| format!("Failed creating Graph HTTP client: {error}"))
}

/// Graph client that returns redirects instead of following them, so a
/// download's redirect to the content server goes through `content_http`.
pub(crate) fn graph_http_client_no_redirect() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(resolve_connect_timeout())
        .timeout(resolve_request_timeout())
        .http2_adaptive_window(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("Failed creating no-redirect Graph HTTP client: {error}"))
}

/// Client for file contents, downloads and uploads. HTTP/1.1 only, so each
/// transfer gets its own TCP connection: over HTTP/2 every parallel transfer
/// to the content server shares one connection and its flow-control window,
/// which held 32 downloads to about 2 MB/s combined on a long-distance link.
/// Idle connections are still kept and reused for the next file.
///
/// No total deadline: a large file on a slow link may take an hour and must
/// not be cut off while data still flows. A transfer fails only when no data
/// arrives for the stall timeout (60 s by default).
pub(crate) fn content_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(resolve_connect_timeout())
        .read_timeout(resolve_stall_timeout())
        .http1_only()
        .build()
        .map_err(|error| format!("Failed creating file transfer HTTP client: {error}"))
}
