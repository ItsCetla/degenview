//! Uploads a snapshot to the host the user configured and returns the link it made.
//!
//! The protocol is small on purpose (it is written down in `docs/snapshots.md`): one `POST` of the
//! PNG as `image/png`, an optional bearer token, and `{"url": "https://…"}` back. One call is one
//! attempt with no redirects, so the address the user approved is the only one the image is sent
//! to; trying again is the caller's choice.

use crate::model::snapshot::{
    MAX_RESPONSE_BYTES, MAX_UPLOAD_BYTES, SharedSnapshot, SnapshotHost, UploadError,
    parse_upload_response,
};
use std::time::Duration;

/// How long a host may take, connecting, taking the image and answering.
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// The longest token that is sent.
const MAX_TOKEN_CHARS: usize = 512;

/// What a host answered.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostResponse {
    pub status: u16,
    /// Seconds from a `Retry-After` header, if it was one.
    pub retry_after: Option<u64>,
    pub body: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportError {
    Timeout,
    Network,
}

/// What goes on the wire.
pub struct Post<'a> {
    pub url: &'a str,
    pub token: Option<&'a str>,
    pub png: &'a [u8],
}

/// The network seam: sends one request. Tests put a local server behind it.
pub trait SnapshotTransport: Send + Sync {
    fn post(&self, post: &Post) -> Result<HostResponse, TransportError>;
}

pub struct UreqTransport {
    agent: ureq::Agent,
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::with_timeout(UPLOAD_TIMEOUT)
    }
}

impl UreqTransport {
    pub fn with_timeout(timeout: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl SnapshotTransport for UreqTransport {
    fn post(&self, post: &Post) -> Result<HostResponse, TransportError> {
        let mut request = self
            .agent
            .post(post.url)
            .header("Content-Type", "image/png")
            .header(
                "User-Agent",
                concat!("DegenView/", env!("CARGO_PKG_VERSION")),
            );
        if let Some(token) = post.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let mut response = request.send(post.png).map_err(|error| match error {
            ureq::Error::Timeout(_) => TransportError::Timeout,
            _ => TransportError::Network,
        })?;
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok());
        let status = response.status().as_u16();
        // A host that sends more than a link is talking about something else: read what fits.
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES as u64)
            .read_to_vec()
            .unwrap_or_default();
        Ok(HostResponse {
            status,
            retry_after,
            body,
        })
    }
}

/// A token that can ride in a header: visible ASCII only, no spaces.
pub fn token_is_valid(token: &str) -> bool {
    !token.is_empty()
        && token.chars().count() <= MAX_TOKEN_CHARS
        && token.chars().all(|c| c.is_ascii_graphic())
}

fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}

/// Uploads `png` to `host` and returns the snapshot it stored.
pub fn upload(
    transport: &dyn SnapshotTransport,
    host: &SnapshotHost,
    token: Option<&str>,
    png: &[u8],
) -> Result<SharedSnapshot, UploadError> {
    if png.len() > MAX_UPLOAD_BYTES {
        return Err(UploadError::TooLarge);
    }
    if !is_png(png) {
        // Whatever this is, it is not what the host was promised.
        return Err(UploadError::BadResponse);
    }
    if token.is_some_and(|token| !token_is_valid(token)) {
        return Err(UploadError::Unauthorized);
    }
    let response = transport
        .post(&Post {
            url: host.endpoint(),
            token,
            png,
        })
        .map_err(|error| match error {
            TransportError::Timeout => UploadError::Timeout,
            TransportError::Network => UploadError::Offline,
        })?;
    match response.status {
        200 | 201 => parse_upload_response(&response.body, host),
        401 | 403 => Err(UploadError::Unauthorized),
        413 => Err(UploadError::RejectedAsTooLarge),
        429 => Err(UploadError::RateLimited {
            retry_after: response.retry_after,
        }),
        // A redirect is never followed: the image goes to the address the user approved only.
        300..=399 => Err(UploadError::BadResponse),
        other => Err(UploadError::Server(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// What a test server saw of one request.
    #[derive(Clone, Debug, Default)]
    struct Seen {
        request_line: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    impl Seen {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        }
    }

    fn read_request(stream: &mut std::net::TcpStream) -> Seen {
        let mut raw = Vec::new();
        let mut buffer = [0u8; 4096];
        let head_end = loop {
            let read = stream.read(&mut buffer).unwrap_or(0);
            if read == 0 {
                break raw.len();
            }
            raw.extend_from_slice(&buffer[..read]);
            if let Some(at) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
                break at + 4;
            }
        };
        let head = String::from_utf8_lossy(&raw[..head_end.min(raw.len())]).into_owned();
        let mut lines = head.split("\r\n");
        let request_line = lines.next().unwrap_or_default().to_owned();
        let headers: Vec<(String, String)> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
            .collect();
        let length = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, v)| v.parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = raw[head_end.min(raw.len())..].to_vec();
        while body.len() < length {
            let read = stream.read(&mut buffer).unwrap_or(0);
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buffer[..read]);
        }
        Seen {
            request_line,
            headers,
            body,
        }
    }

    /// A host on this computer that answers each connection with the next `(status, headers,
    /// body)` and records what it was sent.
    fn serve(
        answers: Vec<(u16, &'static str, &'static str)>,
    ) -> (
        SnapshotHost,
        Arc<Mutex<Vec<Seen>>>,
        std::thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = SnapshotHost::parse(&format!(
            "http://{}/v1/snapshots",
            listener.local_addr().unwrap()
        ))
        .expect("a local host");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        let thread = std::thread::spawn(move || {
            for (status, headers, body) in answers {
                let (mut stream, _) = listener.accept().unwrap();
                log.lock().unwrap().push(read_request(&mut stream));
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (host, seen, thread)
    }

    fn png() -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&[7; 64]);
        bytes
    }

    const LINK: &str = r#"{"url":"http://127.0.0.1:1/s/abc.png"}"#;

    #[test]
    fn an_upload_posts_the_png_with_its_token_and_returns_the_link() {
        let (host, seen, thread) = serve(vec![(201, "", LINK)]);
        let shared = upload(
            &UreqTransport::default(),
            &host,
            Some("s3cret-token"),
            &png(),
        )
        .expect("uploaded");
        thread.join().unwrap();
        assert_eq!(shared.url, "http://127.0.0.1:1/s/abc.png");
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].request_line, "POST /v1/snapshots HTTP/1.1");
        assert_eq!(seen[0].header("content-type"), Some("image/png"));
        assert_eq!(seen[0].header("authorization"), Some("Bearer s3cret-token"));
        assert!(
            seen[0]
                .header("user-agent")
                .unwrap()
                .starts_with("DegenView/")
        );
        assert_eq!(seen[0].body, png());
    }

    #[test]
    fn without_a_token_no_authorization_is_sent() {
        let (host, seen, thread) = serve(vec![(200, "", LINK)]);
        upload(&UreqTransport::default(), &host, None, &png()).expect("uploaded");
        thread.join().unwrap();
        assert_eq!(seen.lock().unwrap()[0].header("authorization"), None);
    }

    #[test]
    fn what_a_host_says_is_told_apart() {
        let cases: Vec<((u16, &'static str, &'static str), UploadError)> = vec![
            ((401, "", ""), UploadError::Unauthorized),
            ((403, "", ""), UploadError::Unauthorized),
            ((413, "", ""), UploadError::RejectedAsTooLarge),
            (
                (429, "Retry-After: 12\r\n", ""),
                UploadError::RateLimited {
                    retry_after: Some(12),
                },
            ),
            (
                (429, "", ""),
                UploadError::RateLimited { retry_after: None },
            ),
            ((500, "", ""), UploadError::Server(500)),
            ((503, "", "busy"), UploadError::Server(503)),
            ((404, "", ""), UploadError::Server(404)),
            ((200, "", "not json"), UploadError::BadResponse),
            (
                (200, "", r#"{"url":"javascript:1"}"#),
                UploadError::BadResponse,
            ),
            ((200, "", "{}"), UploadError::BadResponse),
        ];
        for ((status, headers, body), expected) in cases {
            let (host, _, thread) = serve(vec![(status, headers, body)]);
            assert_eq!(
                upload(&UreqTransport::default(), &host, None, &png()),
                Err(expected),
                "{status} {body}"
            );
            thread.join().unwrap();
        }
    }

    #[test]
    fn a_redirect_is_not_followed() {
        let (host, seen, thread) = serve(vec![(
            302,
            "Location: https://elsewhere.example/up\r\n",
            "",
        )]);
        assert_eq!(
            upload(&UreqTransport::default(), &host, None, &png()),
            Err(UploadError::BadResponse)
        );
        thread.join().unwrap();
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "only the approved address was asked"
        );
    }

    #[test]
    fn a_host_that_is_not_there_is_offline() {
        // A port nothing listens on.
        let closed = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap()
        };
        let host = SnapshotHost::parse(&format!("http://{closed}/up")).unwrap();
        assert_eq!(
            upload(&UreqTransport::default(), &host, None, &png()),
            Err(UploadError::Offline)
        );
    }

    #[test]
    fn a_host_that_does_not_answer_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let host =
            SnapshotHost::parse(&format!("http://{}/up", listener.local_addr().unwrap())).unwrap();
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_request(&mut stream);
            std::thread::sleep(Duration::from_millis(400));
        });
        let transport = UreqTransport::with_timeout(Duration::from_millis(100));
        assert_eq!(
            upload(&transport, &host, None, &png()),
            Err(UploadError::Timeout)
        );
        thread.join().unwrap();
    }

    #[test]
    fn nothing_is_sent_that_the_host_would_not_take() {
        struct Never;
        impl SnapshotTransport for Never {
            fn post(&self, _: &Post) -> Result<HostResponse, TransportError> {
                panic!("nothing should be sent");
            }
        }
        let host = SnapshotHost::parse("https://snap.example.com/up").unwrap();
        let huge = {
            let mut bytes = png();
            bytes.resize(MAX_UPLOAD_BYTES + 1, 0);
            bytes
        };
        assert_eq!(
            upload(&Never, &host, None, &huge),
            Err(UploadError::TooLarge)
        );
        assert_eq!(
            upload(&Never, &host, None, b"GIF89a not a png"),
            Err(UploadError::BadResponse)
        );
        for token in ["has space", "new\nline", "", "tab\t"] {
            assert_eq!(
                upload(&Never, &host, Some(token), &png()),
                Err(UploadError::Unauthorized),
                "{token:?}"
            );
        }
    }

    #[test]
    fn a_token_is_visible_ascii_without_spaces() {
        assert!(token_is_valid("abcDEF123-._~+/="));
        assert!(!token_is_valid(""));
        assert!(!token_is_valid("a b"));
        assert!(!token_is_valid("é"));
        assert!(!token_is_valid(&"a".repeat(MAX_TOKEN_CHARS + 1)));
    }

    #[test]
    fn a_failure_is_not_repeated_by_itself() {
        let (host, seen, thread) = serve(vec![(503, "", "")]);
        assert_eq!(
            upload(&UreqTransport::default(), &host, None, &png()),
            Err(UploadError::Server(503))
        );
        thread.join().unwrap();
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}
