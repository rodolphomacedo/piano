//! Just enough HTTP/1.1 to serve one page and a handful of JSON routes.
//!
//! Hand-written rather than pulled from a crate, for the same reason
//! `piano-wasm/www` has no bundler: the surface actually used here is a
//! request line, a `Content-Length` and a response — and this server only
//! ever binds to `127.0.0.1` (`docs/PARAMETER-STUDIO.md`'s "Remote or
//! multi-user access" non-goal), so it faces one person's own browser, not
//! the open internet.
//!
//! Every read is bounded. A client that sends an endless header line, or
//! promises a body it never finishes, hits a limit rather than growing a
//! buffer without end.

use std::io::{self, BufRead, Read, Write};

/// Longest request line plus headers accepted, in bytes. Generous for a
/// browser's own headers, far short of anything that could exhaust memory.
const MAX_HEAD_BYTES: usize = 8 * 1024;

/// Longest request body accepted, in bytes. The largest thing any route
/// takes is a file path, so this is already orders of magnitude more than
/// needed.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// The one header this server interprets, lowercased for comparison.
const CONTENT_LENGTH: &str = "content-length";

/// One parsed request. Headers beyond `Content-Length` are not kept —
/// nothing here routes on them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Request {
    /// The HTTP method, uppercased.
    pub(crate) method: String,
    /// The path, with any query string stripped.
    pub(crate) path: String,
    /// The body, as received. Empty for methods that carry none.
    pub(crate) body: String,
}

/// Reads one request, or `None` at a clean end of connection.
///
/// # Errors
///
/// Returns an I/O error if the socket fails, or
/// [`io::ErrorKind::InvalidData`] if the request is malformed or exceeds a
/// limit.
pub(crate) fn read_request(reader: &mut impl BufRead) -> io::Result<Option<Request>> {
    let Some(head) = read_head(reader)? else {
        return Ok(None);
    };
    let Some(start_line) = head.first() else {
        return Ok(None);
    };
    let (method, path) = parse_start_line(start_line)?;
    let body = read_body(reader, content_length(&head)?)?;
    Ok(Some(Request { method, path, body }))
}

/// Reads the request line and headers, stopping at the blank line.
/// `Ok(None)` means the peer closed before sending anything.
fn read_head(reader: &mut impl BufRead) -> io::Result<Option<Vec<String>>> {
    let mut lines: Vec<String> = Vec::new();
    let mut remaining = MAX_HEAD_BYTES;
    loop {
        let line = read_line_bounded(reader, remaining)?;
        if line.is_empty() {
            return if lines.is_empty() {
                Ok(None)
            } else {
                Err(invalid("request head ended without a blank line"))
            };
        }
        remaining = remaining
            .checked_sub(line.len())
            .ok_or_else(|| invalid("request head is longer than the accepted limit"))?;
        let trimmed = line.trim_end_matches(['\r', '\n']).to_string();
        if trimmed.is_empty() {
            return Ok(Some(lines));
        }
        lines.push(trimmed);
    }
}

/// Reads up to and including one `\n`, refusing to buffer more than
/// `limit` bytes. An empty result means end of stream.
fn read_line_bounded(reader: &mut impl BufRead, limit: usize) -> io::Result<String> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(limit as u64)
        .read_until(b'\n', &mut bytes)?;
    String::from_utf8(bytes).map_err(|_| invalid("request head is not valid UTF-8"))
}

/// Splits `GET /api/piano?x=1 HTTP/1.1` into method and path, dropping the
/// query string — no route here reads one.
fn parse_start_line(line: &str) -> io::Result<(String, String)> {
    let mut parts = line.split(' ');
    let method = parts
        .next()
        .filter(|method| !method.is_empty())
        .ok_or_else(|| invalid(&format!("request line has no method: {line:?}")))?;
    let target = parts
        .next()
        .ok_or_else(|| invalid(&format!("request line has no path: {line:?}")))?;
    let path = target.split('?').next().unwrap_or(target);
    Ok((method.to_uppercase(), path.to_string()))
}

/// The declared body length, or zero when no `Content-Length` was sent.
fn content_length(head: &[String]) -> io::Result<usize> {
    for line in head.iter().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case(CONTENT_LENGTH) {
            continue;
        }
        let length: usize = value
            .trim()
            .parse()
            .map_err(|_| invalid(&format!("unreadable Content-Length: {:?}", value.trim())))?;
        if length > MAX_BODY_BYTES {
            return Err(invalid(&format!(
                "body of {length} bytes is over the limit"
            )));
        }
        return Ok(length);
    }
    Ok(0)
}

/// Reads exactly `length` bytes of body.
fn read_body(reader: &mut impl BufRead, length: usize) -> io::Result<String> {
    if length == 0 {
        return Ok(String::new());
    }
    let mut bytes = Vec::with_capacity(length);
    reader
        .by_ref()
        .take(length as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() != length {
        return Err(invalid("connection ended mid-body"));
    }
    String::from_utf8(bytes).map_err(|_| invalid("request body is not valid UTF-8"))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

/// One response, ready to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Response {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

impl Response {
    /// A JSON response with an explicit status.
    pub(crate) fn json(status: u16, body: String) -> Self {
        Self {
            status,
            content_type: "application/json; charset=utf-8",
            body: body.into_bytes(),
        }
    }

    /// A response carrying one of the embedded page assets.
    pub(crate) fn asset(content_type: &'static str, body: &str) -> Self {
        Self {
            status: 200,
            content_type,
            body: body.as_bytes().to_vec(),
        }
    }

    /// A plain-text response, used for every error this server reports.
    pub(crate) fn text(status: u16, body: &str) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            body: body.as_bytes().to_vec(),
        }
    }

    /// This response's status code. Only tests inspect it directly —
    /// production code only ever writes a response out, never reads it back.
    #[cfg(test)]
    pub(crate) fn status(&self) -> u16 {
        self.status
    }

    /// Writes the whole response, headers and body.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the socket fails mid-write.
    pub(crate) fn write_to(&self, writer: &mut impl Write) -> io::Result<()> {
        write!(
            writer,
            "HTTP/1.1 {} {}\r\n\
             Content-Type: {}\r\n\
             Content-Length: {}\r\n\
             Cache-Control: no-store\r\n\
             Connection: keep-alive\r\n\r\n",
            self.status,
            reason_phrase(self.status),
            self.content_type,
            self.body.len(),
        )?;
        writer.write_all(&self.body)?;
        writer.flush()
    }
}

/// The reason phrase for the handful of statuses this server sends.
/// Anything unlisted reports as a generic server error rather than having
/// a phrase invented for it.
fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::io::BufReader;

    use super::*;

    fn parse(raw: &str) -> io::Result<Option<Request>> {
        read_request(&mut BufReader::new(raw.as_bytes()))
    }

    #[test]
    fn a_get_with_no_body_parses() {
        let request = parse("GET /api/piano HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .expect("parses")
            .expect("is a request");
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/api/piano");
        assert!(request.body.is_empty());
    }

    #[test]
    fn a_post_body_is_read_exactly_to_its_content_length() {
        let raw = "POST /api/live HTTP/1.1\r\nContent-Length: 9\r\n\r\n{\"a\":1234}trailing";
        let request = parse(raw).expect("parses").expect("is a request");
        assert_eq!(request.body, "{\"a\":1234");
    }

    #[test]
    fn the_query_string_is_dropped_because_no_route_reads_one() {
        let request = parse("GET /api/piano?tab=2 HTTP/1.1\r\n\r\n")
            .expect("parses")
            .expect("is a request");
        assert_eq!(request.path, "/api/piano");
    }

    #[test]
    fn a_lowercase_content_length_header_is_honoured() {
        // Header names are case-insensitive, and a hand-written client is
        // exactly the sort that sends them lowercased.
        let request = parse("POST /api/save HTTP/1.1\r\ncontent-length: 2\r\n\r\n{}")
            .expect("parses")
            .expect("is a request");
        assert_eq!(request.body, "{}");
    }

    #[test]
    fn a_closed_connection_reads_as_no_request_rather_than_an_error() {
        assert_eq!(parse("").expect("is not an error"), None);
    }

    #[test]
    fn a_head_that_never_ends_is_refused_instead_of_buffered_without_bound() {
        let mut raw = String::from("GET / HTTP/1.1\r\n");
        raw.push_str(&"X-Filler: aaaaaaaaaaaaaaaaaaaa\r\n".repeat(1_000));
        let error = parse(&raw).expect_err("is refused");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_body_larger_than_the_limit_is_refused_before_it_is_read() {
        let raw = format!(
            "POST /api/save HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY_BYTES + 1
        );
        let error = parse(&raw).expect_err("is refused");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_body_shorter_than_its_declared_length_is_an_error_not_a_short_read() {
        let raw = "POST /api/live HTTP/1.1\r\nContent-Length: 50\r\n\r\n{}";
        let error = parse(raw).expect_err("is refused");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn an_unreadable_content_length_is_refused() {
        let raw = "POST /api/live HTTP/1.1\r\nContent-Length: banana\r\n\r\n";
        assert!(parse(raw).is_err());
    }

    #[test]
    fn a_response_carries_its_own_length_and_status() {
        let mut written = Vec::new();
        Response::json(200, "{\"ok\":true}".to_string())
            .write_to(&mut written)
            .expect("writes");
        let text = String::from_utf8(written).expect("is UTF-8");
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"), "{text}");
        assert!(text.contains("Content-Length: 11\r\n"), "{text}");
        assert!(text.ends_with("\r\n\r\n{\"ok\":true}"), "{text}");
    }

    #[test]
    fn every_status_this_server_sends_has_its_own_reason_phrase() {
        for status in [200, 400, 404, 405, 503] {
            assert_ne!(
                reason_phrase(status),
                "Internal Server Error",
                "no phrase for {status}"
            );
        }
        assert_eq!(reason_phrase(500), "Internal Server Error");
    }
}
