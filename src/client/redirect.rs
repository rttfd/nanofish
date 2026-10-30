//! Redirect handling shared by the clients that can open new connections.

use core::fmt::Write as _;
use heapless::{String, Vec};
use nanofish::{
    client::io::{HttpEndpoint, parse_endpoint},
    codec,
    error::Error,
    header::{HttpHeader, headers},
    method::HttpMethod,
    protocol::MAX_HEADERS,
    response::HttpResponse,
    status_code::StatusCode,
};

/// Longest redirect target that is followed.
pub const REDIRECT_URL_SIZE: usize = 512;
/// Most path segments kept while resolving a relative redirect.
const MAX_SEGMENTS: usize = 24;

/// Whether `status` is a redirect that carries a `Location` to follow.
pub const fn is_redirect(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MovedPermanently
            | StatusCode::Found
            | StatusCode::SeeOther
            | StatusCode::TemporaryRedirect
            | StatusCode::Other(308)
    )
}

/// The method and body handling for the request that follows a redirect.
///
/// `303 See Other` turns everything but `HEAD` into a `GET`. `301` and `302`
/// turn `POST` into a `GET`, as browsers do. `307` and `308` repeat the
/// request unchanged. A request that becomes a `GET` loses its body.
pub const fn redirected_request(status: StatusCode, method: HttpMethod) -> (HttpMethod, bool) {
    match (status, method) {
        (StatusCode::SeeOther, HttpMethod::HEAD) => (HttpMethod::HEAD, false),
        (StatusCode::SeeOther, _)
        | (StatusCode::MovedPermanently | StatusCode::Found, HttpMethod::POST) => {
            (HttpMethod::GET, false)
        }
        _ => (method, true),
    }
}

/// Whether two endpoints are the same origin (scheme, host and port).
pub fn same_origin(a: &HttpEndpoint<'_>, b: &HttpEndpoint<'_>) -> bool {
    a.scheme == b.scheme && a.port == b.port && a.host.eq_ignore_ascii_case(b.host)
}

/// Resolve a `Location` header value against the URL that was requested,
/// returning the absolute URL to request next.
///
/// Handles absolute URLs, scheme-relative (`//host/path`), absolute-path
/// (`/path`), query-only (`?q`) and relative-path (`../x`, `y`) references.
/// A redirect from `https` to `http` is refused, since it would send a request
/// that was protected by TLS in the clear.
///
/// # Errors
///
/// - [`Error::BufferOverflow`] if the result does not fit in `N` bytes.
/// - [`Error::InvalidUrl`] if the location is not a valid URL.
/// - [`Error::InvalidResponse`] for an `https` to `http` downgrade.
pub fn resolve_location<const N: usize>(
    base: &HttpEndpoint<'_>,
    location: &str,
) -> Result<String<N>, Error> {
    let location = location.trim();
    // The fragment never reaches the server.
    let location = location.split_once('#').map_or(location, |(l, _)| l);
    let mut url = String::new();

    if location.starts_with("http://") || location.starts_with("https://") {
        url.push_str(location).map_err(|_| Error::BufferOverflow)?;
    } else if let Some(rest) = location.strip_prefix("//") {
        write!(url, "{}://{rest}", base.scheme).map_err(|_| Error::BufferOverflow)?;
    } else {
        let authority = base.host_header::<N>()?;
        write!(url, "{}://{authority}", base.scheme).map_err(|_| Error::BufferOverflow)?;
        if location.starts_with('/') {
            url.push_str(location).map_err(|_| Error::BufferOverflow)?;
        } else if location.starts_with('?') {
            url.push_str(base.path).map_err(|_| Error::BufferOverflow)?;
            url.push_str(location).map_err(|_| Error::BufferOverflow)?;
        } else if location.is_empty() {
            return Err(Error::InvalidUrl);
        } else {
            push_relative_path(&mut url, base.path, location)?;
        }
    }

    // Validate the result, and refuse to leave TLS.
    let target = parse_endpoint(&url)?;
    if base.scheme == "https" && target.scheme == "http" {
        return Err(Error::InvalidResponse("Redirect from https to http"));
    }
    Ok(url)
}

/// Append `reference` resolved against the directory of `base_path`, dropping
/// `.` and `..` segments (RFC 3986 section 5.2.4). A query in `reference` is
/// kept.
fn push_relative_path<const N: usize>(
    url: &mut String<N>,
    base_path: &str,
    reference: &str,
) -> Result<(), Error> {
    let (reference_path, query) = reference
        .split_once('?')
        .map_or((reference, None), |(path, query)| (path, Some(query)));
    // Everything up to and including the last `/` of the base path.
    let directory = base_path.rsplit_once('/').map_or("", |(dir, _)| dir);

    let mut segments: Vec<&str, MAX_SEGMENTS> = Vec::new();
    let trailing_slash = reference_path.ends_with('/')
        || matches!(reference_path.rsplit('/').next(), Some("." | ".."));
    for segment in directory.split('/').chain(reference_path.split('/')) {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            segment => segments.push(segment).map_err(|_| Error::BufferOverflow)?,
        }
    }

    for segment in &segments {
        url.push('/').map_err(|_| Error::BufferOverflow)?;
        url.push_str(segment).map_err(|_| Error::BufferOverflow)?;
    }
    if segments.is_empty() || trailing_slash {
        url.push('/').map_err(|_| Error::BufferOverflow)?;
    }
    if let Some(query) = query {
        url.push('?').map_err(|_| Error::BufferOverflow)?;
        url.push_str(query).map_err(|_| Error::BufferOverflow)?;
    }
    Ok(())
}

/// Headers that must not follow a redirect to a different origin.
fn is_sensitive(name: &str) -> bool {
    [headers::AUTHORIZATION, "Cookie", "Proxy-Authorization"]
        .iter()
        .any(|sensitive| name.eq_ignore_ascii_case(sensitive))
}

/// Send a request with `send`, following up to `max_redirects` redirects.
///
/// `send` performs one request and leaves the raw response in the buffer it is
/// given, returning how many bytes it wrote. Each hop reuses `buffer`, so the
/// final response replaces the redirect responses in it. The request tuple is
/// `(method, URL, headers, body)`.
///
/// See [`HttpClientOptions::max_redirects`](nanofish::HttpClientOptions::max_redirects)
/// for how headers, methods and bodies are treated across redirects.
///
/// # Errors
///
/// Returns the errors of `send` and of parsing, [`Error::InvalidResponse`]
/// after more than `max_redirects` redirects, and the errors of
/// [`resolve_location`] for an unusable `Location`.
pub async fn follow<'b, 'h, F>(
    max_redirects: usize,
    request: (HttpMethod, &str, &[HttpHeader<'h>], Option<&[u8]>),
    buffer: &'b mut [u8],
    mut send: F,
) -> Result<(HttpResponse<'b>, usize), Error>
where
    F: AsyncFnMut(
        HttpMethod,
        &str,
        &[HttpHeader<'_>],
        Option<&[u8]>,
        &mut [u8],
    ) -> Result<usize, Error>,
{
    let (mut method, endpoint, headers, mut body) = request;
    let mut next_url: Option<String<REDIRECT_URL_SIZE>> = None;
    let mut sensitive_headers_allowed = true;
    let mut redirects = 0;

    let used = loop {
        let target = next_url.as_deref().unwrap_or(endpoint);

        let mut kept = Vec::<HttpHeader<'h>, MAX_HEADERS>::new();
        let headers = if sensitive_headers_allowed {
            headers
        } else {
            kept.extend(headers.iter().filter(|h| !is_sensitive(h.name)).cloned());
            &kept
        };

        let used = send(method, target, headers, body, &mut *buffer).await?;
        if max_redirects == 0 {
            break used;
        }

        // Decide in a scope of its own: the response borrows the buffer, which
        // the next hop reuses.
        let redirect = {
            let response = codec::parse_resp(&buffer[..used])?;
            if is_redirect(response.status_code) {
                response
                    .get_header(headers::LOCATION)
                    .map(|location| {
                        resolve_location::<REDIRECT_URL_SIZE>(&parse_endpoint(target)?, location)
                            .map(|url| (response.status_code, url))
                    })
                    .transpose()?
            } else {
                None
            }
        };
        let Some((status, url)) = redirect else {
            break used;
        };

        if redirects == max_redirects {
            return Err(Error::InvalidResponse("Too many redirects"));
        }
        redirects += 1;

        if !same_origin(&parse_endpoint(target)?, &parse_endpoint(&url)?) {
            sensitive_headers_allowed = false;
        }
        let (next_method, keep_body) = redirected_request(status, method);
        method = next_method;
        if !keep_body {
            body = None;
        }
        next_url = Some(url);
    };

    Ok((codec::parse_resp(&buffer[..used])?, used))
}

#[cfg(test)]
mod tests {
    use nanofish::client::redirect::*;

    fn resolve(base: &str, location: &str) -> Result<std::string::String, Error> {
        let base = parse_endpoint(base).unwrap();
        resolve_location::<128>(&base, location).map(|url| std::string::String::from(url.as_str()))
    }

    #[test]
    fn test_is_redirect() {
        for status in [
            StatusCode::MovedPermanently,
            StatusCode::Found,
            StatusCode::SeeOther,
            StatusCode::TemporaryRedirect,
            StatusCode::Other(308),
        ] {
            assert!(is_redirect(status), "{status:?}");
        }
        for status in [
            StatusCode::Ok,
            StatusCode::NotModified,
            StatusCode::MultipleChoices,
            StatusCode::NotFound,
        ] {
            assert!(!is_redirect(status), "{status:?}");
        }
    }

    #[test]
    fn test_redirected_request_method_and_body() {
        use HttpMethod::{DELETE, GET, HEAD, POST, PUT};
        let see_other = StatusCode::SeeOther;
        assert_eq!(redirected_request(see_other, POST), (GET, false));
        assert_eq!(redirected_request(see_other, PUT), (GET, false));
        assert_eq!(redirected_request(see_other, GET), (GET, false));
        assert_eq!(redirected_request(see_other, HEAD), (HEAD, false));

        for status in [StatusCode::MovedPermanently, StatusCode::Found] {
            assert_eq!(redirected_request(status, POST), (GET, false));
            assert_eq!(redirected_request(status, PUT), (PUT, true));
            assert_eq!(redirected_request(status, GET), (GET, true));
        }

        for status in [StatusCode::TemporaryRedirect, StatusCode::Other(308)] {
            assert_eq!(redirected_request(status, POST), (POST, true));
            assert_eq!(redirected_request(status, DELETE), (DELETE, true));
        }
    }

    #[test]
    fn test_same_origin() {
        let a = parse_endpoint("http://example.com/a").unwrap();
        assert!(same_origin(
            &a,
            &parse_endpoint("http://EXAMPLE.com/b?x").unwrap()
        ));
        assert!(same_origin(
            &a,
            &parse_endpoint("http://example.com:80/").unwrap()
        ));
        assert!(!same_origin(
            &a,
            &parse_endpoint("http://example.com:8080/").unwrap()
        ));
        assert!(!same_origin(
            &a,
            &parse_endpoint("https://example.com/").unwrap()
        ));
        assert!(!same_origin(
            &a,
            &parse_endpoint("http://other.com/").unwrap()
        ));
    }

    #[test]
    fn test_resolve_absolute_and_scheme_relative() {
        let base = "http://example.com/a/b";
        assert_eq!(
            resolve(base, "http://other.org/x?y=1").unwrap(),
            "http://other.org/x?y=1"
        );
        assert_eq!(
            resolve(base, "https://other.org/").unwrap(),
            "https://other.org/"
        );
        assert_eq!(
            resolve(base, "//cdn.example.net/f").unwrap(),
            "http://cdn.example.net/f"
        );
        // The fragment is dropped.
        assert_eq!(
            resolve(base, "http://other.org/x#top").unwrap(),
            "http://other.org/x"
        );
    }

    #[test]
    fn test_resolve_absolute_path_keeps_port() {
        assert_eq!(
            resolve("http://example.com/a", "/b?c=1").unwrap(),
            "http://example.com/b?c=1"
        );
        assert_eq!(
            resolve("http://example.com:8080/a", "/b").unwrap(),
            "http://example.com:8080/b"
        );
        assert_eq!(
            resolve("https://example.com:443/a", "/b").unwrap(),
            "https://example.com/b"
        );
        assert_eq!(
            resolve("http://[::1]:8080/a", "/b").unwrap(),
            "http://[::1]:8080/b"
        );
    }

    #[test]
    fn test_resolve_relative_paths() {
        let base = "http://example.com/a/b/c?old=1";
        for (location, expected) in [
            ("d", "http://example.com/a/b/d"),
            ("d/e", "http://example.com/a/b/d/e"),
            ("./d", "http://example.com/a/b/d"),
            ("../d", "http://example.com/a/d"),
            ("../../d", "http://example.com/d"),
            ("../../../../d", "http://example.com/d"),
            ("d?x=1", "http://example.com/a/b/d?x=1"),
            ("d/", "http://example.com/a/b/d/"),
            ("..", "http://example.com/a/"),
            (".", "http://example.com/a/b/"),
            ("?q=2", "http://example.com/a/b/c?q=2"),
        ] {
            assert_eq!(resolve(base, location).unwrap(), expected, "{location}");
        }
        // A base without a path resolves against the root.
        assert_eq!(
            resolve("http://example.com", "x").unwrap(),
            "http://example.com/x"
        );
    }

    #[test]
    fn test_resolve_rejects_downgrade_and_garbage() {
        assert!(matches!(
            resolve("https://example.com/a", "http://example.com/b"),
            Err(Error::InvalidResponse("Redirect from https to http"))
        ));
        // Scheme-relative keeps the scheme, so https stays https.
        assert_eq!(
            resolve("https://example.com/a", "//example.com/b").unwrap(),
            "https://example.com/b"
        );
        // An upgrade is fine.
        assert!(resolve("http://example.com/a", "https://example.com/b").is_ok());
        assert!(matches!(
            resolve("http://example.com/a", ""),
            Err(Error::InvalidUrl)
        ));
        assert!(matches!(
            resolve("http://example.com/a", "http://"),
            Err(Error::InvalidUrl)
        ));
    }

    #[test]
    fn test_resolve_overflow() {
        let base = parse_endpoint("http://example.com/a").unwrap();
        assert!(matches!(
            resolve_location::<16>(&base, "/a-very-long-path-that-does-not-fit"),
            Err(Error::BufferOverflow)
        ));
    }

    /// One request seen by the fake transport.
    #[derive(Debug, PartialEq)]
    struct Call {
        method: HttpMethod,
        url: std::string::String,
        headers: std::vec::Vec<std::string::String>,
        body: Option<std::vec::Vec<u8>>,
    }

    /// Run `follow` against canned `responses` (the last one repeats).
    fn run(
        max: usize,
        method: HttpMethod,
        url: &str,
        headers: &[HttpHeader<'_>],
        body: Option<&[u8]>,
        responses: &[&[u8]],
    ) -> (
        Result<(StatusCode, std::string::String), Error>,
        std::vec::Vec<Call>,
    ) {
        let mut calls = std::vec::Vec::new();
        let mut buffer = [0u8; 512];
        let mut next = 0;
        let result = futures_lite::future::block_on(follow(
            max,
            (method, url, headers, body),
            &mut buffer,
            async |method: HttpMethod,
                   url: &str,
                   headers: &[HttpHeader<'_>],
                   body: Option<&[u8]>,
                   buffer: &mut [u8]| {
                calls.push(Call {
                    method,
                    url: url.into(),
                    headers: headers.iter().map(|h| h.name.into()).collect(),
                    body: body.map(<[u8]>::to_vec),
                });
                let raw = responses[next.min(responses.len() - 1)];
                next += 1;
                buffer[..raw.len()].copy_from_slice(raw);
                Ok(raw.len())
            },
        ))
        .map(|(response, _)| {
            (
                response.status_code,
                response.body.as_str().unwrap_or("").into(),
            )
        });
        (result, calls)
    }

    const FOUND_B: &[u8] = b"HTTP/1.1 302 Found\r\nLocation: /b\r\nContent-Length: 0\r\n\r\n";
    const OK_DONE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ndone";

    #[test]
    fn test_follow_disabled_returns_the_redirect() {
        let (result, calls) = run(
            0,
            HttpMethod::GET,
            "http://h/a",
            &[],
            None,
            &[FOUND_B, OK_DONE],
        );
        assert_eq!(result.unwrap().0, StatusCode::Found);
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn test_follow_redirect_to_final_response() {
        let (result, calls) = run(
            3,
            HttpMethod::GET,
            "http://h:8080/a",
            &[],
            None,
            &[FOUND_B, OK_DONE],
        );
        assert_eq!(result.unwrap(), (StatusCode::Ok, "done".into()));
        let urls: std::vec::Vec<_> = calls.iter().map(|c| c.url.as_str()).collect();
        assert_eq!(urls, ["http://h:8080/a", "http://h:8080/b"]);
    }

    #[test]
    fn test_follow_chain_of_redirects() {
        let to_c: &[u8] = b"HTTP/1.1 301 Moved\r\nlocation: https://h/c\r\n\r\n";
        let (result, calls) = run(
            3,
            HttpMethod::GET,
            "http://h/a",
            &[],
            None,
            &[FOUND_B, to_c, OK_DONE],
        );
        assert_eq!(result.unwrap().0, StatusCode::Ok);
        let urls: std::vec::Vec<_> = calls.iter().map(|c| c.url.as_str()).collect();
        assert_eq!(urls, ["http://h/a", "http://h/b", "https://h/c"]);
    }

    #[test]
    fn test_follow_post_becomes_get_on_302_and_303() {
        for raw in [
            FOUND_B,
            &b"HTTP/1.1 303 See Other\r\nLocation: /b\r\n\r\n"[..],
        ] {
            let (result, calls) = run(
                2,
                HttpMethod::POST,
                "http://h/a",
                &[],
                Some(b"data"),
                &[raw, OK_DONE],
            );
            assert_eq!(result.unwrap().0, StatusCode::Ok);
            assert_eq!(calls[0].method, HttpMethod::POST);
            assert_eq!(calls[0].body.as_deref(), Some(b"data".as_slice()));
            assert_eq!(calls[1].method, HttpMethod::GET);
            assert_eq!(calls[1].body, None);
        }
    }

    #[test]
    fn test_follow_307_and_308_repeat_the_request() {
        for raw in [
            &b"HTTP/1.1 307 Temporary Redirect\r\nLocation: /b\r\n\r\n"[..],
            b"HTTP/1.1 308 Permanent Redirect\r\nLocation: /b\r\n\r\n",
        ] {
            let (result, calls) = run(
                2,
                HttpMethod::POST,
                "http://h/a",
                &[],
                Some(b"data"),
                &[raw, OK_DONE],
            );
            assert_eq!(result.unwrap().0, StatusCode::Ok);
            assert_eq!(calls[1].method, HttpMethod::POST);
            assert_eq!(calls[1].body.as_deref(), Some(b"data".as_slice()));
        }
    }

    #[test]
    fn test_follow_drops_credentials_when_leaving_the_origin() {
        let headers = [
            HttpHeader::authorization("Bearer t"),
            HttpHeader::new("Cookie", "a=b"),
            HttpHeader::new("Proxy-Authorization", "x"),
            HttpHeader::new("Accept", "text/plain"),
        ];
        let elsewhere: &[u8] = b"HTTP/1.1 302 Found\r\nLocation: http://other/x\r\n\r\n";
        let back: &[u8] = b"HTTP/1.1 302 Found\r\nLocation: http://h/y\r\n\r\n";
        let (result, calls) = run(
            4,
            HttpMethod::GET,
            "http://h/a",
            &headers,
            None,
            &[elsewhere, back, OK_DONE],
        );
        assert_eq!(result.unwrap().0, StatusCode::Ok);
        assert_eq!(calls[0].headers.len(), 4);
        // Once credentials were dropped they stay dropped, even on the way back.
        assert_eq!(calls[1].headers, ["Accept"]);
        assert_eq!(calls[2].headers, ["Accept"]);
    }

    #[test]
    fn test_follow_keeps_credentials_within_the_origin() {
        let headers = [HttpHeader::authorization("Bearer t")];
        let (result, calls) = run(
            2,
            HttpMethod::GET,
            "http://h/a",
            &headers,
            None,
            &[FOUND_B, OK_DONE],
        );
        assert_eq!(result.unwrap().0, StatusCode::Ok);
        assert_eq!(calls[1].headers, ["Authorization"]);
    }

    #[test]
    fn test_follow_gives_up_after_max_redirects() {
        let (result, calls) = run(2, HttpMethod::GET, "http://h/a", &[], None, &[FOUND_B]);
        assert!(matches!(
            result,
            Err(Error::InvalidResponse("Too many redirects"))
        ));
        // The original request plus two followed redirects.
        assert_eq!(calls.len(), 3);
    }

    #[test]
    fn test_follow_returns_redirect_without_location() {
        let raw: &[u8] = b"HTTP/1.1 302 Found\r\nContent-Length: 0\r\n\r\n";
        let (result, calls) = run(3, HttpMethod::GET, "http://h/a", &[], None, &[raw]);
        assert_eq!(result.unwrap().0, StatusCode::Found);
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn test_follow_refuses_downgrade_and_bad_location() {
        let downgrade: &[u8] = b"HTTP/1.1 302 Found\r\nLocation: http://h/x\r\n\r\n";
        let (result, _) = run(3, HttpMethod::GET, "https://h/a", &[], None, &[downgrade]);
        assert!(matches!(
            result,
            Err(Error::InvalidResponse("Redirect from https to http"))
        ));

        let bad: &[u8] = b"HTTP/1.1 302 Found\r\nLocation: http://\r\n\r\n";
        let (result, _) = run(3, HttpMethod::GET, "http://h/a", &[], None, &[bad]);
        assert!(matches!(result, Err(Error::InvalidUrl)));
    }

    #[test]
    fn test_follow_propagates_transport_errors() {
        let mut buffer = [0u8; 64];
        let result = futures_lite::future::block_on(follow(
            3,
            (HttpMethod::GET, "http://h/", &[], None),
            &mut buffer,
            async |_, _, _, _, _: &mut [u8]| Err(Error::NoResponse),
        ));
        assert!(matches!(result, Err(Error::NoResponse)));
    }
}
