//! Bounded HTTP/1 connections for the phone server. Own the accepted socket
//! so read/write deadlines apply to slow clients as well as normal requests.
//! Parsing and chunk decoding use the same small, established building blocks
//! as other HTTP implementations; responses are still written by tiny_http.

use std::io::{self, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tiny_http::{Header, Method};

pub struct Request {
    method: Method,
    url: String,
    headers: Vec<Header>,
    length: Option<usize>,
    body: Box<dyn Read + Send>,
}

impl Request {
    pub fn method(&self) -> &Method {
        &self.method
    }
    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn headers(&self) -> &[Header] {
        &self.headers
    }
    pub fn body_length(&self) -> Option<usize> {
        self.length
    }
    pub fn as_reader(&mut self) -> &mut dyn Read {
        &mut self.body
    }

    pub fn read(stream: &mut TcpStream) -> io::Result<Self> {
        stream.set_read_timeout(Some(Duration::from_secs(15)))?;
        stream.set_write_timeout(Some(Duration::from_secs(15)))?;
        let mut reader = BufReader::new(DeadlineReader {
            stream: stream.try_clone()?,
            deadline: Instant::now() + Duration::from_secs(60),
            // Also bounds chunk framing overhead, before the decoded-body cap.
            remaining: 27 * 1024 * 1024,
        });
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            if head.len() == 32 * 1024 {
                return Err(invalid("headers too large"));
            }
            let mut byte = [0];
            reader.read_exact(&mut byte)?;
            head.push(byte[0]);
        }
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut parsed = httparse::Request::new(&mut headers);
        if !parsed
            .parse(&head)
            .map_err(|_| invalid("invalid request"))?
            .is_complete()
        {
            return Err(invalid("incomplete request"));
        }
        let method = parsed
            .method
            .unwrap_or("")
            .parse::<Method>()
            .map_err(|_| invalid("invalid method"))?;
        let url = parsed.path.unwrap_or("").to_string();
        if !url.starts_with('/') {
            return Err(invalid("expected an origin-relative target"));
        }
        let mut headers = Vec::new();
        let mut length = None;
        let mut chunked = false;
        for h in parsed.headers.iter() {
            let value = std::str::from_utf8(h.value)
                .map_err(|_| invalid("invalid header"))?
                .trim();
            if h.name.eq_ignore_ascii_case("content-length") {
                if length.is_some() {
                    return Err(invalid("duplicate content length"));
                }
                length = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| invalid("invalid content length"))?,
                );
            }
            if h.name.eq_ignore_ascii_case("transfer-encoding") {
                if chunked || !value.eq_ignore_ascii_case("chunked") {
                    return Err(invalid("unsupported transfer encoding"));
                }
                chunked = true;
            }
            headers
                .push(Header::from_bytes(h.name, h.value).map_err(|_| invalid("invalid header"))?);
        }
        if chunked && length.is_some() {
            return Err(invalid("ambiguous body length"));
        }
        if headers.iter().any(|h| {
            h.field.equiv("Expect") && h.value.as_str().eq_ignore_ascii_case("100-continue")
        }) {
            stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
        }
        let body: Box<dyn Read + Send> = if chunked {
            Box::new(ChunkedBody(chunked_transfer::Decoder::new(reader)))
        } else {
            Box::new(ExactBody {
                reader,
                remaining: length.unwrap_or(0),
            })
        };
        Ok(Self {
            method,
            url,
            headers,
            length,
            body,
        })
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

struct DeadlineReader {
    stream: TcpStream,
    deadline: Instant,
    remaining: usize,
}
impl Read for DeadlineReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || self.remaining == 0 {
            return Err(invalid("request limit exceeded"));
        }
        self.stream
            .set_read_timeout(Some(left.min(Duration::from_secs(15))))?;
        let length = bytes.len().min(self.remaining);
        let read = self.stream.read(&mut bytes[..length])?;
        self.remaining -= read;
        Ok(read)
    }
}

struct ChunkedBody<R: Read>(chunked_transfer::Decoder<R>);
impl<R: Read> Read for ChunkedBody<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let read = self.0.read(bytes)?;
        // The decoder can return EOF while still inside a chunk. Do not
        // acknowledge a disconnected upload as a successfully saved prefix.
        if read == 0 && self.0.remaining_chunks_size().is_some_and(|left| left > 0) {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete chunk",
            ));
        }
        Ok(read)
    }
}

struct ExactBody<R> {
    reader: R,
    remaining: usize,
}
impl<R: Read> Read for ExactBody<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 || bytes.is_empty() {
            return Ok(0);
        }
        let length = bytes.len().min(self.remaining);
        let read = self.reader.read(&mut bytes[..length])?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete body",
            ));
        }
        self.remaining -= read;
        Ok(read)
    }
}

#[cfg(test)]
impl From<tiny_http::TestRequest> for Request {
    fn from(value: tiny_http::TestRequest) -> Self {
        let mut request: tiny_http::Request = value.into();
        let mut body = Vec::new();
        request.as_reader().read_to_end(&mut body).unwrap();
        Self {
            method: request.method().clone(),
            url: request.url().into(),
            headers: request.headers().to_vec(),
            length: Some(body.len()),
            body: Box::new(io::Cursor::new(body)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incomplete_chunks_fail_and_complete_chunks_decode() {
        let mut partial = ChunkedBody(chunked_transfer::Decoder::new(&b"5\r\nabc"[..]));
        assert!(partial.read_to_end(&mut Vec::new()).is_err());
        let mut full = ChunkedBody(chunked_transfer::Decoder::new(
            &b"3\r\nabc\r\n0\r\n\r\n"[..],
        ));
        let mut bytes = Vec::new();
        full.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"abc");
    }

    #[test]
    fn truncated_declared_bodies_fail() {
        let mut body = ExactBody {
            reader: &b"short"[..],
            remaining: 10,
        };
        assert!(body.read_to_end(&mut Vec::new()).is_err());
    }
    #[test]
    fn deadline_is_enforced_before_reading_more_bytes() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let mut reader = DeadlineReader {
            stream,
            deadline: Instant::now(),
            remaining: 100,
        };
        assert!(reader.read(&mut [0; 1]).is_err());
    }
}
