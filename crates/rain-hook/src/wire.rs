//! The HTTP side of the hook: the request it writes and the answer it reads. Only what the
//! backend of Rain sends is understood; anything else is "no answer", and "no answer" means the
//! hook does nothing (fail open).

/// What the backend answered.
#[derive(Debug, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    pub body: Vec<u8>,
}

/// One HTTP/1.1 request that the backend's pipe server accepts. `Connection: close` makes the
/// server end the answer by closing the pipe, so the reader needs no length bookkeeping.
pub fn request(envelope: &str) -> Vec<u8> {
    format!(
        "POST /api/v1/internal/hook HTTP/1.1\r\nHost: rain\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        envelope.len(),
        envelope
    )
    .into_bytes()
}

/// Reads the status and the body from the raw bytes that came back. `None` when the bytes are
/// not an HTTP answer (too short, no status line, a body that is cut).
pub fn parse_answer(raw: &[u8]) -> Option<Answer> {
    let head_end = find(raw, b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..head_end]).ok()?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next()?;
    let mut parts = status_line.split(' ');
    if !parts.next()?.starts_with("HTTP/1.") {
        return None;
    }
    let status: u16 = parts.next()?.parse().ok()?;

    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    for line in lines {
        let (name, value) = line.split_once(':')?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.parse().ok();
        } else if name.eq_ignore_ascii_case("transfer-encoding") && value.contains("chunked") {
            chunked = true;
        }
    }
    let rest = &raw[head_end + 4..];
    let body = if chunked {
        dechunk(rest)?
    } else if let Some(length) = content_length {
        // A body shorter than it says was cut by a closing pipe: not an answer.
        rest.get(..length)?.to_vec()
    } else {
        rest.to_vec()
    };
    Some(Answer { status, body })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Joins the pieces of a body sent with `Transfer-Encoding: chunked`.
fn dechunk(mut rest: &[u8]) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let line_end = find(rest, b"\r\n")?;
        let size_text = std::str::from_utf8(&rest[..line_end]).ok()?;
        let size = usize::from_str_radix(size_text.split(';').next()?.trim(), 16).ok()?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            return Some(body);
        }
        body.extend_from_slice(rest.get(..size)?);
        rest = rest.get(size + 2..)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_names_the_route_and_the_exact_length() {
        let text = String::from_utf8(request("{\"a\":1}")).unwrap();
        assert!(text.starts_with("POST /api/v1/internal/hook HTTP/1.1\r\n"));
        assert!(text.contains("Content-Length: 7\r\n"));
        assert!(text.ends_with("\r\n\r\n{\"a\":1}"));
    }

    #[test]
    fn an_answer_with_a_length_is_read() {
        let raw =
            b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\n\r\n{}";
        assert_eq!(
            parse_answer(raw),
            Some(Answer {
                status: 200,
                body: b"{}".to_vec()
            })
        );
    }

    #[test]
    fn an_answer_without_a_body_is_read() {
        let raw = b"HTTP/1.1 204 No Content\r\ncache-control: no-store\r\n\r\n";
        assert_eq!(
            parse_answer(raw),
            Some(Answer {
                status: 204,
                body: Vec::new()
            })
        );
    }

    #[test]
    fn a_chunked_answer_is_joined() {
        let raw =
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
        assert_eq!(parse_answer(raw).unwrap().body, b"abcde");
    }

    #[test]
    fn garbage_and_cut_answers_are_not_answers() {
        assert_eq!(parse_answer(b""), None);
        assert_eq!(parse_answer(b"not http at all"), None);
        assert_eq!(
            parse_answer(b"HTTP/1.1 200 OK\r\ncontent-length: 10\r\n\r\nabc"),
            None
        );
        assert_eq!(parse_answer(b"HTTP/1.1 abc\r\n\r\n"), None);
    }
}
