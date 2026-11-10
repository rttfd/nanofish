//! Streaming response bodies: hand a body to a sink piece by piece instead of
//! holding all of it in the response buffer.

use nanofish::{error::Error, response::HttpResponse, status_code::StatusCode};

/// Receives a response as it is read by
/// [`HttpClient::request_streaming`](nanofish::HttpClient::request_streaming).
///
/// The body arrives in pieces no larger than the response buffer, already
/// decoded: a chunked body is delivered as its payload, without chunk framing.
/// A sink can write to flash, feed a parser or a hash, and so on.
#[expect(
    async_fn_in_trait,
    reason = "no_std embedded executors are single-threaded, so no `Send` bound is wanted"
)]
pub trait BodySink {
    /// Called once, when the status line and headers have been read and
    /// before any body data.
    ///
    /// Return an error to abandon the response, for example for a status
    /// other than `200` or a `Content-Length` that does not fit; the error is
    /// returned by `request_streaming`.
    async fn head(&mut self, response: &HttpResponse<'_>) -> Result<(), Error>;

    /// Called with the next piece of the body, never with an empty slice.
    ///
    /// Return an error to stop reading; it is returned by `request_streaming`.
    async fn chunk(&mut self, data: &[u8]) -> Result<(), Error>;
}

/// What [`HttpClient::request_streaming`](nanofish::HttpClient::request_streaming)
/// learned about a response whose body was streamed to a [`BodySink`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamedResponse {
    /// The status code of the response.
    pub status_code: StatusCode,
    /// How many body bytes were delivered to the sink.
    pub body_len: usize,
}

#[cfg(test)]
mod tests {
    use nanofish::codec::{self, ChunkDecoder};
    use nanofish::error::Error;

    /// Decode `raw` in pieces of `step` bytes, returning the payload.
    fn decode(raw: &[u8], step: usize) -> Result<std::vec::Vec<u8>, Error> {
        let mut decoder = ChunkDecoder::new();
        let mut payload = std::vec::Vec::new();
        for piece in raw.chunks(step) {
            let mut rest = piece;
            while !rest.is_empty() && !decoder.is_done() {
                let step = decoder.step(rest)?;
                if let Some(range) = step.data {
                    payload.extend_from_slice(&rest[range]);
                }
                rest = &rest[step.consumed..];
            }
        }
        if decoder.is_done() {
            Ok(payload)
        } else {
            Err(Error::InvalidResponse("Incomplete"))
        }
    }

    const BODIES: [&[u8]; 6] = [
        b"5\r\nhello\r\n0\r\n\r\n",
        b"3\r\nabc\r\n4;ext=1\r\nwxyz\r\n0\r\nT: v\r\nU: w\r\n\r\n",
        b"7\r\nab0\r\n\r\n\r\n0\r\n\r\n",
        b"a\r\n0123456789\r\n0\r\n\r\n",
        b"1\r\nx\r\n1\r\ny\r\n1\r\nz\r\n0\r\n\r\n",
        b"0\r\n\r\n",
    ];

    #[test]
    fn test_decoder_matches_dechunk_for_every_split() {
        for raw in BODIES {
            // Reference result: the in-place decoder on a response around it.
            let mut response = std::vec::Vec::from(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".as_slice(),
            );
            let head_len = response.len();
            response.extend_from_slice(raw);
            let len = response.len();
            let total = codec::dechunk(&mut response, len).unwrap();
            let expected = &response[head_len..total];

            for step in 1..=raw.len() {
                assert_eq!(decode(raw, step).unwrap(), expected, "{raw:?} step {step}");
            }
        }
    }

    #[test]
    fn test_decoder_reports_payload_ranges_in_each_piece() {
        let mut decoder = ChunkDecoder::new();
        let input = b"5\r\nhello\r\n0\r\n\r\n";
        let step = decoder.step(input).unwrap();
        assert_eq!(step.data, Some(3..8));
        assert_eq!(step.consumed, 8);
        assert!(!decoder.is_done());
        let step = decoder.step(&input[step.consumed..]).unwrap();
        assert_eq!(step.data, None);
        assert_eq!(step.consumed, input.len() - 8);
        assert!(decoder.is_done());
    }

    #[test]
    fn test_decoder_incomplete_body_is_not_done() {
        for raw in [
            &b"5\r\nhel"[..],
            b"5\r\nhello",
            b"5\r\nhello\r\n0\r\n",
            b"5",
        ] {
            assert!(decode(raw, 3).is_err(), "{raw:?}");
        }
    }

    #[test]
    fn test_decoder_rejects_malformed_framing() {
        for (raw, message) in [
            (&b"zz\r\nhello\r\n0\r\n\r\n"[..], "Invalid chunk size"),
            (b"\r\n", "Invalid chunk size"),
            (b"ffffffffffffffffff\r\n", "Invalid chunk size"),
            (b"5\r\nhelloXX0\r\n\r\n", "Invalid chunk terminator"),
            (b"5\r\nhello\n", "Invalid chunk terminator"),
        ] {
            assert!(
                matches!(decode(raw, 2), Err(Error::InvalidResponse(m)) if m == message),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn test_decoder_rejects_overlong_size_line() {
        let mut raw = std::vec::Vec::new();
        raw.extend_from_slice(b"5;");
        raw.extend(core::iter::repeat_n(b'a', 100));
        raw.extend_from_slice(b"\r\nhello\r\n0\r\n\r\n");
        assert!(matches!(
            decode(&raw, 7),
            Err(Error::InvalidResponse("Chunk size line too long"))
        ));
    }

    #[test]
    fn test_decoder_stops_at_end_of_body() {
        // Bytes after the final empty line are not consumed.
        let mut decoder = ChunkDecoder::new();
        let input = b"0\r\n\r\nNEXT";
        let step = decoder.step(input).unwrap();
        assert!(decoder.is_done());
        assert_eq!(step.consumed, 5);
    }
}
