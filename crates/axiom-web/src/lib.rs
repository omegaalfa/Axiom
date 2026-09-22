//! Bounded, SSRF-aware HTTP/HTTPS fetching for future read-only tools.

use reqwest::blocking::Client;
use std::{
    fmt,
    io::Read,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::Arc,
    time::Duration,
};
use url::Url;

pub const MAX_DOWNLOAD_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_CONTENT_BYTES: usize = 512 * 1024;
pub const MAX_REDIRECTS: usize = 3;
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchUrlRequest {
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchUrlOutput {
    pub url: String,
    pub final_url: String,
    pub content_type: String,
    pub content: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchUrlError {
    InvalidUrl,
    UnsupportedScheme(String),
    BlockedAddress(String),
    Timeout,
    TooLarge { limit: usize },
    UnsupportedContentType(String),
    UnsupportedEncoding(String),
    HttpStatus(u16),
    Network(String),
    Cancelled,
    RedirectLimit,
}

impl fmt::Display for FetchUrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl => write!(f, "invalid URL"),
            Self::UnsupportedScheme(scheme) => write!(f, "unsupported URL scheme: {scheme}"),
            Self::BlockedAddress(address) => write!(f, "blocked network address: {address}"),
            Self::Timeout => write!(f, "request timed out"),
            Self::TooLarge { limit } => write!(f, "response exceeds {limit} bytes"),
            Self::UnsupportedContentType(kind) => write!(f, "unsupported content type: {kind}"),
            Self::UnsupportedEncoding(encoding) => write!(f, "unsupported encoding: {encoding}"),
            Self::HttpStatus(status) => write!(f, "HTTP status {status}"),
            Self::Network(message) => write!(f, "network error: {message}"),
            Self::Cancelled => write!(f, "request cancelled"),
            Self::RedirectLimit => write!(f, "redirect limit reached"),
        }
    }
}
impl std::error::Error for FetchUrlError {}

#[derive(Clone, Debug)]
struct RawResponse {
    status: u16,
    content_type: Option<String>,
    location: Option<String>,
    body: Vec<u8>,
}

trait FetchTransport: Send + Sync {
    fn request(
        &self,
        url: &Url,
        address: SocketAddr,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<RawResponse, FetchUrlError>;
}

#[derive(Default)]
struct ReqwestTransport;

impl FetchTransport for ReqwestTransport {
    fn request(
        &self,
        url: &Url,
        address: SocketAddr,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<RawResponse, FetchUrlError> {
        if cancelled() {
            return Err(FetchUrlError::Cancelled);
        }
        let host = url.host_str().ok_or(FetchUrlError::InvalidUrl)?;
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .resolve(host, address)
            .build()
            .map_err(|error| FetchUrlError::Network(error.to_string()))?;
        let mut response = client.get(url.clone()).send().map_err(|error| {
            if error.is_timeout() {
                FetchUrlError::Timeout
            } else {
                FetchUrlError::Network(error.to_string())
            }
        })?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let mut body = Vec::new();
        let mut limited = (&mut response).take((MAX_DOWNLOAD_BYTES + 1) as u64);
        let mut buffer = [0u8; 8192];
        loop {
            if cancelled() {
                return Err(FetchUrlError::Cancelled);
            }
            let read = limited
                .read(&mut buffer)
                .map_err(|error| FetchUrlError::Network(error.to_string()))?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buffer[..read]);
            if body.len() > MAX_DOWNLOAD_BYTES {
                return Err(FetchUrlError::TooLarge {
                    limit: MAX_DOWNLOAD_BYTES,
                });
            }
        }
        Ok(RawResponse {
            status,
            content_type,
            location,
            body,
        })
    }
}

#[derive(Clone)]
pub struct FetchUrlCapability {
    transport: Arc<dyn FetchTransport>,
}

impl fmt::Debug for FetchUrlCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FetchUrlCapability").finish_non_exhaustive()
    }
}

impl Default for FetchUrlCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl FetchUrlCapability {
    pub fn new() -> Self {
        Self {
            transport: Arc::new(ReqwestTransport),
        }
    }

    #[cfg(test)]
    fn with_transport(transport: Arc<dyn FetchTransport>) -> Self {
        Self { transport }
    }

    pub fn fetch_url(&self, request: FetchUrlRequest) -> Result<FetchUrlOutput, FetchUrlError> {
        self.fetch_url_with_cancel(request, || false)
    }

    pub fn fetch_url_with_cancel<F>(
        &self,
        request: FetchUrlRequest,
        cancelled: F,
    ) -> Result<FetchUrlOutput, FetchUrlError>
    where
        F: Fn() -> bool,
    {
        let original = parse_supported_url(&request.url)?;
        let mut current = original.clone();
        for redirect in 0..=MAX_REDIRECTS {
            if cancelled() {
                return Err(FetchUrlError::Cancelled);
            }
            let address = resolve_public_address(&current)?;
            let response = self.transport.request(&current, address, &cancelled)?;
            if (300..400).contains(&response.status) {
                if redirect == MAX_REDIRECTS {
                    return Err(FetchUrlError::RedirectLimit);
                }
                let location = response
                    .location
                    .ok_or(FetchUrlError::HttpStatus(response.status))?;
                current = current
                    .join(&location)
                    .map_err(|_| FetchUrlError::InvalidUrl)?;
                parse_supported_url(current.as_str())?;
                continue;
            }
            if !(200..300).contains(&response.status) {
                return Err(FetchUrlError::HttpStatus(response.status));
            }
            if response.body.len() > MAX_DOWNLOAD_BYTES {
                return Err(FetchUrlError::TooLarge {
                    limit: MAX_DOWNLOAD_BYTES,
                });
            }
            let content_type = supported_content_type(response.content_type.as_deref())?;
            let content = String::from_utf8(response.body)
                .map_err(|_| FetchUrlError::UnsupportedEncoding("utf-8".into()))?;
            let truncated = content.len() > MAX_CONTENT_BYTES;
            let content = truncate_utf8(content, MAX_CONTENT_BYTES);
            return Ok(FetchUrlOutput {
                url: original.to_string(),
                final_url: current.to_string(),
                content_type,
                content,
                truncated,
            });
        }
        Err(FetchUrlError::RedirectLimit)
    }
}

fn parse_supported_url(value: &str) -> Result<Url, FetchUrlError> {
    let url = Url::parse(value).map_err(|_| FetchUrlError::InvalidUrl)?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(FetchUrlError::InvalidUrl);
    }
    match url.scheme() {
        "http" | "https" => Ok(url),
        scheme => Err(FetchUrlError::UnsupportedScheme(scheme.into())),
    }
}

fn truncate_utf8(mut content: String, max_bytes: usize) -> String {
    if content.len() <= max_bytes {
        return content;
    }
    let mut end = max_bytes;
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    content.truncate(end);
    content
}

fn supported_content_type(value: Option<&str>) -> Result<String, FetchUrlError> {
    let value = value.unwrap_or("text/plain");
    let kind = value
        .split(';')
        .next()
        .unwrap_or(value)
        .trim()
        .to_ascii_lowercase();
    match kind.as_str() {
        "text/plain" | "application/json" | "text/html" => Ok(kind),
        _ => Err(FetchUrlError::UnsupportedContentType(kind)),
    }
}

fn resolve_public_address(url: &Url) -> Result<SocketAddr, FetchUrlError> {
    let host = url.host_str().ok_or(FetchUrlError::InvalidUrl)?;
    let port = url
        .port_or_known_default()
        .ok_or(FetchUrlError::InvalidUrl)?;
    let addresses: Vec<_> = match url.host() {
        Some(url::Host::Ipv4(ip)) => vec![SocketAddr::new(IpAddr::V4(ip), port)],
        Some(url::Host::Ipv6(ip)) => vec![SocketAddr::new(IpAddr::V6(ip), port)],
        Some(url::Host::Domain(domain)) => (domain, port)
            .to_socket_addrs()
            .map_err(|error| FetchUrlError::Network(error.to_string()))?
            .collect(),
        None => return Err(FetchUrlError::InvalidUrl),
    };
    if addresses.is_empty()
        || addresses.iter().any(|address| {
            is_blocked_address(address.ip())
                || host.eq_ignore_ascii_case("localhost")
                || host.eq_ignore_ascii_case("localhost.")
        })
    {
        return Err(FetchUrlError::BlockedAddress(host.into()));
    }
    Ok(addresses[0])
}

fn is_blocked_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.octets() == [169, 254, 169, 254]
                || ip.octets() == [169, 254, 170, 2]
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| is_blocked_address(IpAddr::V4(mapped)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[derive(Default)]
    struct ScriptedTransport {
        responses: Mutex<VecDeque<Result<RawResponse, FetchUrlError>>>,
        seen: Mutex<Vec<String>>,
    }
    impl FetchTransport for ScriptedTransport {
        fn request(
            &self,
            url: &Url,
            _: SocketAddr,
            _: &dyn Fn() -> bool,
        ) -> Result<RawResponse, FetchUrlError> {
            self.seen.lock().unwrap().push(url.to_string());
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err(FetchUrlError::Network("no scripted response".into())))
        }
    }
    fn capability(
        responses: Vec<Result<RawResponse, FetchUrlError>>,
    ) -> (FetchUrlCapability, Arc<ScriptedTransport>) {
        let transport = Arc::new(ScriptedTransport {
            responses: Mutex::new(responses.into()),
            ..Default::default()
        });
        (
            FetchUrlCapability::with_transport(transport.clone()),
            transport,
        )
    }
    fn response(status: u16, content_type: &str, body: &str) -> Result<RawResponse, FetchUrlError> {
        Ok(RawResponse {
            status,
            content_type: Some(content_type.into()),
            location: None,
            body: body.as_bytes().into(),
        })
    }
    #[test]
    fn parses_schemes_and_rejects_unsupported() {
        assert!(parse_supported_url("https://example.com/x").is_ok());
        assert!(parse_supported_url("http://example.com/x").is_ok());
        assert!(matches!(
            parse_supported_url("file:///tmp/a"),
            Err(FetchUrlError::UnsupportedScheme(_))
        ));
        assert!(matches!(
            parse_supported_url("ftp://example.com"),
            Err(FetchUrlError::UnsupportedScheme(_))
        ));
        assert!(matches!(
            parse_supported_url("not a url"),
            Err(FetchUrlError::InvalidUrl)
        ));
    }
    #[test]
    fn blocks_private_and_special_addresses() {
        for value in [
            "localhost",
            "localhost.",
            "127.0.0.1",
            "::1",
            "10.0.0.1",
            "169.254.1.1",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            let host = if value.contains(':') {
                format!("[{value}]")
            } else {
                value.to_owned()
            };
            let url = format!("http://{host}/");
            assert!(
                matches!(
                    resolve_public_address(&Url::parse(&url).unwrap()),
                    Err(FetchUrlError::BlockedAddress(_))
                ),
                "{value}"
            );
        }
    }
    #[test]
    fn fetches_supported_text_json_html_and_unicode() {
        for (kind, body) in [
            ("text/plain", "Olá"),
            ("application/json; charset=utf-8", "{\"ok\":true}"),
            ("text/html; charset=utf-8", "<h1>Olá</h1>"),
        ] {
            let (capability, _) = capability(vec![response(200, kind, body)]);
            let output = capability
                .fetch_url(FetchUrlRequest {
                    url: "https://example.com/doc".into(),
                })
                .unwrap();
            assert_eq!(output.content_type, kind.split(';').next().unwrap());
            assert_eq!(output.content, body);
        }
    }

    #[test]
    fn content_limit_is_bounded_on_utf8_boundary() {
        let body = "á".repeat(MAX_CONTENT_BYTES);
        let (cap, _) = capability(vec![response(200, "text/plain", &body)]);
        let output = cap
            .fetch_url(FetchUrlRequest {
                url: "https://example.com".into(),
            })
            .unwrap();
        assert!(output.truncated);
        assert!(output.content.len() <= MAX_CONTENT_BYTES);
        assert!(output.content.is_char_boundary(output.content.len()));
    }

    #[test]
    fn propagates_timeout_from_transport() {
        let (cap, _) = capability(vec![Err(FetchUrlError::Timeout)]);
        assert_eq!(
            cap.fetch_url(FetchUrlRequest {
                url: "https://example.com".into(),
            }),
            Err(FetchUrlError::Timeout)
        );
    }
    #[test]
    fn rejects_binary_status_and_limits() {
        let (cap, _) = capability(vec![response(200, "application/octet-stream", "x")]);
        assert!(matches!(
            cap.fetch_url(FetchUrlRequest {
                url: "https://example.com".into()
            }),
            Err(FetchUrlError::UnsupportedContentType(_))
        ));
        let (cap, _) = capability(vec![response(404, "text/plain", "missing")]);
        assert_eq!(
            cap.fetch_url(FetchUrlRequest {
                url: "https://example.com".into()
            }),
            Err(FetchUrlError::HttpStatus(404))
        );
        let oversized = RawResponse {
            status: 200,
            content_type: Some("text/plain".into()),
            location: None,
            body: vec![b'x'; MAX_DOWNLOAD_BYTES + 1],
        };
        let (cap, _) = capability(vec![Ok(oversized)]);
        assert_eq!(
            cap.fetch_url(FetchUrlRequest {
                url: "https://example.com".into()
            }),
            Err(FetchUrlError::TooLarge {
                limit: MAX_DOWNLOAD_BYTES
            })
        );
        let (cap, _) = capability(vec![Ok(RawResponse {
            status: 200,
            content_type: Some("text/plain; charset=latin1".into()),
            location: None,
            body: vec![0xff],
        })]);
        assert_eq!(
            cap.fetch_url(FetchUrlRequest {
                url: "https://example.com".into()
            }),
            Err(FetchUrlError::UnsupportedEncoding("utf-8".into()))
        );
    }
    #[test]
    fn follows_bounded_redirects_and_blocks_redirect_target() {
        let (cap, transport) = capability(vec![Ok(RawResponse {
            status: 302,
            content_type: None,
            location: Some("http://127.0.0.1/".into()),
            body: Vec::new(),
        })]);
        assert!(matches!(
            cap.fetch_url(FetchUrlRequest {
                url: "https://example.com".into()
            }),
            Err(FetchUrlError::BlockedAddress(_))
        ));
        assert_eq!(transport.seen.lock().unwrap().len(), 1);

        let redirects = (0..=MAX_REDIRECTS)
            .map(|_| {
                Ok(RawResponse {
                    status: 302,
                    content_type: None,
                    location: Some("https://example.com/next".into()),
                    body: Vec::new(),
                })
            })
            .collect();
        let (capability, _) = capability(redirects);
        assert_eq!(
            capability.fetch_url(FetchUrlRequest {
                url: "https://example.com".into()
            }),
            Err(FetchUrlError::RedirectLimit)
        );
    }
    #[test]
    fn cancellation_is_checked_before_transport() {
        let (capability, transport) = capability(vec![response(200, "text/plain", "ok")]);
        assert_eq!(
            capability.fetch_url_with_cancel(
                FetchUrlRequest {
                    url: "https://example.com".into()
                },
                || true
            ),
            Err(FetchUrlError::Cancelled)
        );
        assert!(transport.seen.lock().unwrap().is_empty());
    }
}
