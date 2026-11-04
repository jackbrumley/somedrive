use super::*;
use std::collections::BTreeSet;
use std::os::unix::fs::FileExt;

// Large files download in parallel byte ranges and resume after an
// interruption. Microsoft's file server limits each connection to a couple
// of MB/s, so one big file on one connection crawls; several ranges at once
// use the line. Finished ranges are recorded next to a pre-sized partial
// file, so a stall, a 429 or a restart only fetches what is missing.

/// Files at least this big download in ranges and can resume.
pub(crate) const SEGMENTED_DOWNLOAD_MIN_BYTES: u64 = 64 * 1024 * 1024;
const SEGMENT_BYTES: u64 = 16 * 1024 * 1024;
/// Ranges of one file fetched at once.
const SEGMENT_CONNECTIONS: usize = 8;
/// Tries per range before the whole download is handed back for a retry.
const SEGMENT_ATTEMPTS: u32 = 4;
/// Partial files and their range records start with this; the local scan
/// leaves them alone (see `is_resumable_partial_name`).
pub(crate) const PARTIAL_DOWNLOAD_PREFIX: &str = ".somedrive-partial-";
/// A partial nobody resumed for this long is removed by the local scan.
pub(crate) const PARTIAL_DOWNLOAD_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// What a ranged download ended with.
pub(crate) enum SegmentedDownload {
    /// Every range is in place and the file matches OneDrive's hash.
    Complete { data_path: PathBuf, bytes: u64 },
    /// The file server's link stopped working; ask Graph for a new one.
    LinkExpired,
    /// Microsoft answered 429; the finished ranges are kept.
    Throttled { retry_after: Duration },
    /// The file server ignored the range request; download in one piece.
    RangesUnsupported,
    /// Something else failed; the finished ranges are kept.
    Failed(String),
}

enum SegmentError {
    LinkExpired,
    Throttled(Duration),
    RangesUnsupported,
    Cancelled,
    Failed(String),
}

/// The partial file and its range record for this item and cloud version.
/// The version is part of the name, so a changed file never resumes from
/// another version's bytes.
pub(crate) fn partial_download_paths(
    local_path: &Path,
    item_id: &str,
    version: &CloudVersion,
) -> (PathBuf, PathBuf) {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(item_id, &mut hasher);
    std::hash::Hash::hash(&version.quick_xor_hash, &mut hasher);
    std::hash::Hash::hash(&version.etag, &mut hasher);
    std::hash::Hash::hash(&version.size, &mut hasher);
    let key = format!("{:016x}", std::hash::Hasher::finish(&hasher));
    let parent = local_path.parent().unwrap_or_else(|| Path::new("."));
    (
        parent.join(format!("{PARTIAL_DOWNLOAD_PREFIX}{key}.part")),
        parent.join(format!("{PARTIAL_DOWNLOAD_PREFIX}{key}.ranges")),
    )
}

/// Removes a partial download and its range record.
pub(crate) fn discard_partial_download(data_path: &Path, ranges_path: &Path) {
    let _ = std::fs::remove_file(data_path);
    let _ = std::fs::remove_file(ranges_path);
}

/// Opens (or creates, pre-sized) the partial file and reads which ranges
/// are already done. A partial of the wrong size is started again.
fn open_partial(
    data_path: &Path,
    ranges_path: &Path,
    total: u64,
) -> Result<(std::fs::File, BTreeSet<u64>), String> {
    let existing_size = std::fs::metadata(data_path)
        .map(|metadata| metadata.len())
        .ok();
    if existing_size == Some(total) {
        let done = std::fs::read_to_string(ranges_path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.trim().parse::<u64>().ok())
            .collect();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(data_path)
            .map_err(|error| format!("Failed opening partial download: {error}"))?;
        return Ok((file, done));
    }
    discard_partial_download(data_path, ranges_path);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(data_path)
        .map_err(|error| format!("Failed creating partial download: {error}"))?;
    file.set_len(total)
        .map_err(|error| format!("Failed sizing partial download: {error}"))?;
    Ok((file, BTreeSet::new()))
}

/// Records a finished range, after its bytes are on disk.
fn record_range_done(file: &std::fs::File, ranges_path: &Path, index: u64) -> Result<(), String> {
    file.sync_data()
        .map_err(|error| format!("Failed flushing partial download: {error}"))?;
    let mut record = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(ranges_path)
        .map_err(|error| format!("Failed opening range record: {error}"))?;
    writeln!(record, "{index}").map_err(|error| format!("Failed writing range record: {error}"))?;
    record
        .sync_data()
        .map_err(|error| format!("Failed flushing range record: {error}"))
}

pub(crate) fn segment_count(total: u64) -> u64 {
    total.div_ceil(SEGMENT_BYTES)
}

fn segment_bounds(index: u64, total: u64) -> (u64, u64) {
    let start = index * SEGMENT_BYTES;
    let end_inclusive = (start + SEGMENT_BYTES).min(total) - 1;
    (start, end_inclusive)
}

/// Downloads `content_url` (the file server link from Graph's redirect) in
/// parallel ranges into the partial file for this version, resuming any
/// ranges already done, and checks the result against OneDrive's hash.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn download_in_segments(
    graph: &GraphContext,
    job_id: Option<i64>,
    transfer_id: &Option<String>,
    relative_path: &str,
    item_id: &str,
    version: &CloudVersion,
    content_url: &str,
    local_path: &Path,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<SegmentedDownload, String> {
    let total = version.size;
    let (data_path, ranges_path) = partial_download_paths(local_path, item_id, version);
    if let Some(parent) = data_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Failed creating '{}': {error}", parent.display()))?;
    }
    let (file, done) = open_partial(&data_path, &ranges_path, total)?;
    let count = segment_count(total);
    let resumed_bytes: u64 = done
        .iter()
        .map(|index| {
            let (start, end) = segment_bounds(*index, total);
            end - start + 1
        })
        .sum();
    log::info!(
        "{} [cycle:{}] DOWNLOAD_SEGMENTED path={} bytes={} ranges={} already_done={} connections={}",
        graph.account_prefix,
        graph.cycle_id,
        relative_path,
        total,
        count,
        done.len(),
        SEGMENT_CONNECTIONS
    );

    let bytes_done = AtomicU64Counter::new(resumed_bytes);
    let report_progress = |bytes: u64| {
        if let Some(active_transfer_id) = transfer_id {
            runtime_update_transfer_progress(
                &graph.sync_runtime,
                &graph.profile_id,
                active_transfer_id,
                bytes,
                Some(total),
            );
        }
    };
    report_progress(resumed_bytes);

    let pending: Vec<u64> = (0..count).filter(|index| !done.contains(index)).collect();
    let mut fetches = futures_util::stream::iter(pending.into_iter().map(|index| {
        fetch_range(
            &graph.content_http,
            content_url,
            &file,
            index,
            total,
            cancel_flag,
            &bytes_done,
            &report_progress,
        )
    }))
    .buffer_unordered(SEGMENT_CONNECTIONS);

    while let Some(result) = fetches.next().await {
        let index = match result {
            Ok(index) => index,
            Err(error) => {
                drop(fetches);
                return Ok(match error {
                    SegmentError::LinkExpired => SegmentedDownload::LinkExpired,
                    SegmentError::Throttled(retry_after) => {
                        SegmentedDownload::Throttled { retry_after }
                    }
                    SegmentError::RangesUnsupported => {
                        discard_partial_download(&data_path, &ranges_path);
                        SegmentedDownload::RangesUnsupported
                    }
                    SegmentError::Cancelled => return Err(SYNC_CANCELLED_ERROR.to_string()),
                    SegmentError::Failed(reason) => SegmentedDownload::Failed(reason),
                });
            }
        };
        record_range_done(&file, &ranges_path, index)?;
        if let Some(active_job_id) = job_id {
            let _ = update_download_job_progress(
                &graph.profile_id,
                active_job_id,
                bytes_done.get(),
                Some(total),
            );
        }
    }
    drop(fetches);
    drop(file);

    // The whole file is checked, however it was assembled.
    if let Some(expected) = version.quick_xor_hash.as_deref() {
        let actual = quick_xor_hash_file(&data_path)?;
        if actual != expected {
            discard_partial_download(&data_path, &ranges_path);
            return Ok(SegmentedDownload::Failed(format!(
                "Assembled content hash {actual} does not match OneDrive's {expected}"
            )));
        }
    }
    let _ = std::fs::remove_file(&ranges_path);
    log::info!(
        "{} [cycle:{}] DOWNLOAD_SEGMENTED_COMPLETE path={} bytes={} resumed_bytes={}",
        graph.account_prefix,
        graph.cycle_id,
        relative_path,
        total,
        resumed_bytes
    );
    Ok(SegmentedDownload::Complete {
        data_path,
        bytes: total,
    })
}

/// A byte counter shared by the range fetches of one file.
struct AtomicU64Counter(std::sync::atomic::AtomicU64);

impl AtomicU64Counter {
    fn new(value: u64) -> Self {
        Self(std::sync::atomic::AtomicU64::new(value))
    }
    fn add(&self, value: u64) -> u64 {
        self.0.fetch_add(value, Ordering::Relaxed) + value
    }
    fn sub(&self, value: u64) {
        self.0.fetch_sub(value, Ordering::Relaxed);
    }
    fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Fetches one range into place, retrying it a few times on network and
/// server errors. Returns the range's index when its bytes are written.
#[allow(clippy::too_many_arguments)]
async fn fetch_range(
    client: &reqwest::Client,
    content_url: &str,
    file: &std::fs::File,
    index: u64,
    total: u64,
    cancel_flag: &Arc<AtomicBool>,
    bytes_done: &AtomicU64Counter,
    report_progress: &(dyn Fn(u64) + Sync),
) -> Result<u64, SegmentError> {
    let (start, end_inclusive) = segment_bounds(index, total);
    let mut last_error = String::new();
    for attempt in 1..=SEGMENT_ATTEMPTS {
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(SegmentError::Cancelled);
        }
        if attempt > 1 {
            tokio::time::sleep(exponential_backoff_delay(attempt - 1)).await;
        }
        let response = match client
            .get(content_url)
            .header(
                reqwest::header::RANGE,
                format!("bytes={start}-{end_inclusive}"),
            )
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                last_error = format!("Range request failed: {error}");
                continue;
            }
        };
        let status = response.status();
        match status.as_u16() {
            206 => {}
            200 => return Err(SegmentError::RangesUnsupported),
            429 => {
                let retry_after = parse_retry_after_delay(response.headers())
                    .unwrap_or_else(|| exponential_backoff_delay(attempt));
                return Err(SegmentError::Throttled(retry_after));
            }
            401 | 403 | 404 | 410 => return Err(SegmentError::LinkExpired),
            _ => {
                last_error = format!("Range request answered {status}");
                continue;
            }
        }

        let mut offset = start;
        let mut written: u64 = 0;
        let mut stream = response.bytes_stream();
        let mut stream_error = None;
        while let Some(chunk) = stream.next().await {
            if cancel_flag.load(Ordering::Relaxed) {
                bytes_done.sub(written);
                return Err(SegmentError::Cancelled);
            }
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    stream_error = Some(format!("Range stream failed: {error}"));
                    break;
                }
            };
            if offset + chunk.len() as u64 > end_inclusive + 1 {
                stream_error = Some("Range answer is longer than asked".to_string());
                break;
            }
            file.write_all_at(&chunk, offset)
                .map_err(|error| SegmentError::Failed(format!("Failed writing range: {error}")))?;
            offset += chunk.len() as u64;
            written += chunk.len() as u64;
            report_progress(bytes_done.add(chunk.len() as u64));
        }
        if stream_error.is_none() && offset == end_inclusive + 1 {
            return Ok(index);
        }
        // An incomplete range is fetched again from its start.
        bytes_done.sub(written);
        last_error = stream_error.unwrap_or_else(|| "Range answer ended early".to_string());
    }
    Err(SegmentError::Failed(last_error))
}

#[cfg(test)]
mod segment_tests {
    use super::*;

    #[test]
    fn ranges_cover_the_file_exactly() {
        let total = 3 * SEGMENT_BYTES + 5;
        assert_eq!(segment_count(total), 4);
        assert_eq!(segment_bounds(0, total), (0, SEGMENT_BYTES - 1));
        assert_eq!(segment_bounds(3, total), (3 * SEGMENT_BYTES, total - 1));
        assert_eq!(segment_count(SEGMENT_BYTES), 1);
    }

    #[test]
    fn a_partial_resumes_only_at_the_right_size() {
        let folder = std::env::temp_dir().join(format!(
            "somedrive-partial-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&folder).expect("folder");
        let data = folder.join("data.part");
        let ranges = folder.join("data.ranges");

        let (file, done) = open_partial(&data, &ranges, 1000).expect("create");
        assert!(done.is_empty());
        record_range_done(&file, &ranges, 2).expect("record");
        drop(file);

        let (_, done) = open_partial(&data, &ranges, 1000).expect("resume");
        assert_eq!(done.into_iter().collect::<Vec<_>>(), vec![2]);

        let (_, done) = open_partial(&data, &ranges, 2000).expect("different size");
        assert!(done.is_empty(), "a partial of another size starts again");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn partial_names_follow_the_version() {
        let local = Path::new("/sync/Docs/big.iso");
        let mut version = CloudVersion {
            etag: Some("e1".to_string()),
            ctag: None,
            quick_xor_hash: Some("h1".to_string()),
            size: 100,
        };
        let (first, _) = partial_download_paths(local, "item", &version);
        version.quick_xor_hash = Some("h2".to_string());
        let (second, _) = partial_download_paths(local, "item", &version);
        assert_ne!(first, second);
        let name = first
            .file_name()
            .and_then(|name| name.to_str())
            .expect("name");
        assert!(is_resumable_partial_name(name));
        assert!(
            !is_download_temp_name(name),
            "the scan's temp cleanup leaves it alone"
        );
    }

    /// A one-request-per-connection HTTP server for `body`: answers a Range
    /// request with 206 and those bytes, or ignores ranges when asked to.
    fn serve(body: Vec<u8>, honour_ranges: bool) -> String {
        use std::io::{BufRead, BufReader};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut range = None;
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        let (start, end) = value.trim().split_once('-').expect("range");
                        range = Some((
                            start.parse::<usize>().expect("start"),
                            end.parse::<usize>().expect("end"),
                        ));
                    }
                }
                let (status, bytes) = match range.filter(|_| honour_ranges) {
                    Some((start, end)) => ("206 Partial Content", &body[start..=end]),
                    None => ("200 OK", &body[..]),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(bytes);
            }
        });
        format!("http://{address}/file")
    }

    fn scratch_file(label: &str, size: u64) -> (PathBuf, std::fs::File) {
        let path = std::env::temp_dir().join(format!(
            "somedrive-range-{label}-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .expect("file");
        file.set_len(size).expect("size");
        (path, file)
    }

    #[test]
    fn a_range_lands_at_its_offset() {
        let body: Vec<u8> = (0..1000u32).map(|value| (value % 251) as u8).collect();
        let url = serve(body.clone(), true);
        let (path, file) = scratch_file("ok", body.len() as u64);
        let client = content_http_client().expect("client");
        let cancel = Arc::new(AtomicBool::new(false));
        let counter = AtomicU64Counter::new(0);
        let result = tauri::async_runtime::block_on(fetch_range(
            &client,
            &url,
            &file,
            0,
            body.len() as u64,
            &cancel,
            &counter,
            &|_| {},
        ));
        assert!(matches!(result, Ok(0)));
        assert_eq!(counter.get(), 1000);
        assert_eq!(std::fs::read(&path).expect("read"), body);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_server_that_ignores_ranges_is_detected() {
        let url = serve(vec![7u8; 500], false);
        let (path, file) = scratch_file("unsupported", 500);
        let client = content_http_client().expect("client");
        let cancel = Arc::new(AtomicBool::new(false));
        let counter = AtomicU64Counter::new(0);
        let result = tauri::async_runtime::block_on(fetch_range(
            &client,
            &url,
            &file,
            0,
            500,
            &cancel,
            &counter,
            &|_| {},
        ));
        assert!(matches!(result, Err(SegmentError::RangesUnsupported)));
        let _ = std::fs::remove_file(&path);
    }
}
