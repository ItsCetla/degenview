//! Shared transport for idempotent market-data GETs. Provider parsing and rate limits
//! remain at their call sites; this only bounds network work and classifies failures.

use std::sync::LazyLock;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(12);
const MAX_BODY_BYTES: u64 = 8 * 1024 * 1024;

static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build();
    ureq::Agent::new_with_config(config)
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Retry {
    /// The caller owns backoff, or another request would exceed a provider's limit.
    Never,
    /// One extra attempt for transport failures and 408/425/5xx responses.
    Transient,
}

#[derive(Debug)]
pub enum Error {
    Status(u16),
    Network(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status(code) => write!(f, "HTTP {code}"),
            Self::Network(message) => f.write_str(message),
        }
    }
}

pub fn get_text(url: &str, headers: &[(&str, &str)], retry: Retry) -> Result<String, Error> {
    get_with(&AGENT, url, headers, retry, |_| {
        std::thread::sleep(Duration::from_millis(250))
    })
}

fn get_with(
    agent: &ureq::Agent,
    url: &str,
    headers: &[(&str, &str)],
    retry: Retry,
    sleep: impl Fn(Duration),
) -> Result<String, Error> {
    let attempts = if retry == Retry::Transient { 2 } else { 1 };
    for attempt in 0..attempts {
        let mut request = agent.get(url);
        for &(name, value) in headers {
            request = request.header(name, value);
        }
        let result = match request.call() {
            Ok(mut response) => response
                .body_mut()
                .with_config()
                .limit(MAX_BODY_BYTES)
                .read_to_string()
                .map_err(|error| Error::Network(error.to_string())),
            Err(ureq::Error::StatusCode(code)) => Err(Error::Status(code)),
            Err(error) => Err(Error::Network(error.to_string())),
        };
        let should_retry = matches!(
            result,
            Err(Error::Status(408 | 425 | 500..=599)) | Err(Error::Network(_))
        );
        if attempt + 1 == attempts || !should_retry {
            return result;
        }
        sleep(Duration::from_millis(250));
    }
    unreachable!("at least one attempt")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn serve(statuses: &'static [u16]) -> (String, Arc<AtomicUsize>, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let count = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&count);
        let thread = std::thread::spawn(move || {
            for &status in statuses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 1024];
                let _ = stream.read(&mut request);
                seen.fetch_add(1, Ordering::SeqCst);
                let body = if status == 200 { "ok" } else { "error" };
                write!(stream, "HTTP/1.1 {status} test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        (url, count, thread)
    }

    #[test]
    fn transient_server_error_gets_one_more_attempt() {
        let (url, count, thread) = serve(&[503, 200]);
        let agent =
            ureq::Agent::new_with_config(ureq::Agent::config_builder().max_redirects(0).build());
        assert_eq!(
            get_with(&agent, &url, &[], Retry::Transient, |_| {}).unwrap(),
            "ok"
        );
        thread.join().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn rate_limit_is_left_to_caller() {
        let (url, count, thread) = serve(&[429]);
        let agent = ureq::Agent::new_with_defaults();
        assert!(matches!(
            get_with(&agent, &url, &[], Retry::Transient, |_| {}),
            Err(Error::Status(429))
        ));
        thread.join().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
