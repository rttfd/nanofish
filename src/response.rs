use crate::error::Error;
use crate::socket::SocketWrite;
use crate::status_code::StatusCode;

const CONTENT_LENGTH_PLACEHOLDER_SIZE: usize = usize::ilog10(usize::MAX) as usize + 1;
const CHUNK_LENGTH_PLACEHOLDER_SIZE: usize = usize::BITS as usize / 4;

/// The response builder stages
pub mod stages {
    /// Marker types for the builder stages
    pub struct NotCreated;
    /// Marker type for building status stage
    pub struct BuildStatus;
    /// Marker type for building body stage
    pub struct BuildHeader;

    /// A simple marker type for building chunked body stage
    pub struct BuildChankedBody;
    /// A simple marker type for building chunked body with trailer stage
    pub struct BuildChankedBodyWithTrailer;

    /// A simple marker type for trailer stage
    pub struct Trailer;
}
/// The response type representing an HTTP response.
/// This struct is deliberately empty as response is being sent in a streaming manner using HttpResponseBuilder.
pub struct HttpResponse(core::marker::PhantomData<()>);

impl HttpResponse {
    /// Creates a new HttpResponse instance.
    const fn new() -> Self {
        HttpResponse(core::marker::PhantomData)
    }
}

/// HTTP Response Builder for constructing HTTP responses in a staged manner.
pub struct HttpResponseBuilder<'a, WriteSocket: SocketWrite, Stage = stages::NotCreated> {
    base: BuilderBase<'a, WriteSocket>,
    _phantom: core::marker::PhantomData<Stage>,
}

struct BuilderBase<'a, WriteSocket: SocketWrite> {
    write_socket: &'a mut WriteSocket,
}

impl<'a, WriteSocket: SocketWrite> HttpResponseBuilder<'a, WriteSocket, stages::NotCreated> {
    /// Creates a new HttpResponseBuilder with the provided buffer.
    pub fn new(http_socket: &'a mut WriteSocket) -> HttpResponseBuilder<'a, WriteSocket, stages::BuildStatus> {
        HttpResponseBuilder {
            base: BuilderBase {
                write_socket: http_socket,
            },
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<'buffer, WriteSocket: SocketWrite> HttpResponseBuilder<'buffer, WriteSocket, stages::BuildStatus> {
    /// Adds a header to the HTTP response.
    pub async fn with_status(
        mut self,
        status_code: StatusCode,
    ) -> Result<HttpResponseBuilder<'buffer, WriteSocket, stages::BuildHeader>, Error> {
        // Write "HTTP/1.1 "
        self.base.extend_from_slice(b"HTTP/1.1 ").await?;
        // Write status code in decimal
        extend_from_decimal(&mut self.base.write_socket, status_code.as_u16() as usize).await?;
        self.base.extend_from_slice(b" ").await?;
        // Write " <reason>\r\n"
        self.base.extend_from_slice(status_code.text().as_bytes()).await?;
        self.base.new_line().await?;

        Ok(HttpResponseBuilder {
            base: self.base,
            _phantom: core::marker::PhantomData,
        })
    }

    /// Creates a preflight response with CORS-like PNA headers.
    pub async fn preflight_response(self) -> Result<HttpResponse, Error> {
        self.with_status(StatusCode::NoContent)
            .await?
            .with_cors_like_pna_headers()
            .await?
            .with_no_body()
            .await
    }
}

impl<'buffer, WriteSocket: SocketWrite> HttpResponseBuilder<'buffer, WriteSocket, stages::BuildHeader> {
    /// Adds a header to the HTTP response.
    #[inline(always)]
    pub async fn add_header(&mut self, name: &str, value: &str) -> Result<(), Error> {
        self.add_header_from_slice(name, value.as_bytes()).await
    }

    /// Adds a header to the HTTP response using a byte slice for the value.
    pub async fn add_header_from_slice(&mut self, name: &str, value: &[u8]) -> Result<(), Error> {
        // Write header name
        self.write_header_name(name).await?;
        // Write header value
        self.write_header_value(value).await?;
        // Finalize header line
        self.new_line().await?;

        Ok(())
    }

    /// Adds a header to the HTTP response and returns self for chaining.
    pub async fn with_header(mut self, name: &str, value: &str) -> Result<Self, Error> {
        self.add_header(name, value).await?;
        Ok(self)
    }

    /// Adds a header to the HTTP response using a byte slice for the value and returns self for chaining.
    pub async fn with_header_from_slice(mut self, name: &str, value: &[u8]) -> Result<Self, Error> {
        self.add_header_from_slice(name, value).await?;
        Ok(self)
    }

    /// Finalizes response.
    #[inline(always)]
    pub async fn with_no_body(mut self) -> Result<HttpResponse, Error> {
        self.add_header("Content-Length", "0").await?;
        self.new_line().await?;
        self.base.flush().await?;
        Ok(HttpResponse::new())
    }

    /// Prepares the builder to add a chunked body to the HTTP response.
    pub async fn with_chanked_body(
        mut self,
    ) -> Result<HttpResponseBuilder<'buffer, WriteSocket, stages::BuildChankedBody>, Error> {
        self.add_header("Transfer-Encoding", "chunked").await?;
        self.new_line().await?;
        Ok(HttpResponseBuilder {
            base: self.base,
            _phantom: core::marker::PhantomData,
        })
    }

    /// Prepares the builder to add a chunked body with trailer to the HTTP response.
    pub async fn chanked_body_and_trailer(
        mut self,
        trailer: &str,
    ) -> Result<HttpResponseBuilder<'buffer, WriteSocket, stages::BuildChankedBodyWithTrailer>, Error> {
        self.add_header("Transfer-Encoding", "chunked").await?;
        self.add_header("Trailer", trailer).await?;
        self.new_line().await?;

        Ok(HttpResponseBuilder {
            base: self.base,
            _phantom: core::marker::PhantomData,
        })
    }

    /// Prepares the builder to add a binary body to the HTTP response.
    pub async fn with_body_from_slice(mut self, s: &[u8]) -> Result<HttpResponse, Error> {
        // Prepare space for Content-Length header
        self.write_header_name("Content-Length").await?;

        let mut len_str = [b'0'; CONTENT_LENGTH_PLACEHOLDER_SIZE];
        write_decimal_to_buffer(&mut len_str, s.len()).unwrap();
        self.write_header_value(&len_str).await?;

        self.new_line().await?;
        self.new_line().await?;

        self.base.extend_from_slice(s).await?;
        self.base.flush().await?;
        Ok(HttpResponse::new())
    }

    /// Prepares the builder to add a text body to the HTTP response.
    pub async fn with_body_from_str(self, s: &str) -> Result<HttpResponse, Error> {
        self.with_body_from_slice(s.as_bytes()).await
    }

    /// Prepares the builder to add a plain text body to the HTTP response.
    pub async fn with_plain_text_body(self, s: &str) -> Result<HttpResponse, Error> {
        self.with_header("Content-Type", "text/plain; charset=utf-8")
            .await?
            .with_body_from_str(s)
            .await
    }

    /// Prepares the builder to add a JSON body to the HTTP response.
    pub async fn with_auto_close_connection(mut self) -> Result<Self, Error> {
        // If auto-close connection is set, add the header
        self.base.extend_from_str("Connection: close\r\n").await?;
        Ok(self)
    }

    /// Adds CORS-like PNA headers to the HTTP response.
    /// These headers allow cross-origin requests and private network access.
    pub async fn with_cors_like_pna_headers(self) -> Result<Self, Error> {
        self.with_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
            .await?
            .with_header("Access-Control-Allow-Origin", "*")
            .await?
            .with_header("Access-Control-Allow-Private-Network", "true")
            .await?
            .with_header("Access-Control-Allow-Headers", "Content-Type")
            .await?
            .with_header("Access-Control-Allow-Credentials", "true")
            .await
    }

    /// Creates the response out of compressed HTML page
    /// # Note:s
    /// The page data must be in HTML format compressed with gzip algorithm.
    /// No check is performed to verify the format.
    ///
    pub async fn with_compressed_page(self, page_data: &[u8]) -> Result<HttpResponse, Error> {
        self.with_header("Content-Encoding", "gzip")
            .await?
            .with_header("Content-Type", "text/html; charset=utf-8")
            .await?
            .with_body_from_slice(page_data)
            .await
    }

    /// Creates the response out of HTML page
    /// # Note:
    /// The page data must be in HTML format.
    /// No check is performed to verify the format.
    ///
    pub async fn with_page(self, page_data: &[u8]) -> Result<HttpResponse, Error> {
        self.with_header("Content-Type", "text/html; charset=utf-8")
            .await?
            .with_body_from_slice(page_data)
            .await
    }

    async fn write_header_name(&mut self, name: &str) -> Result<(), Error> {
        self.base.extend_from_str(name).await?;
        self.base.extend_from_str(": ").await
    }

    #[inline(always)]
    async fn write_header_value(&mut self, value: &[u8]) -> Result<(), Error> {
        self.base.extend_from_slice(value).await
    }

    #[inline(always)]
    async fn new_line(&mut self) -> Result<(), Error> {
        self.base.new_line().await
    }
}

impl<'buffer, WriteSocket: SocketWrite> HttpResponseBuilder<'buffer, WriteSocket, stages::BuildChankedBody> {
    /// Adds a chunk to the chunked body.
    pub async fn with_chunk(mut self, chunk: &[u8]) -> Result<Self, Error> {
        // Write chunk size in hexadecimal
        let chank_length_str = [b'0'; CHUNK_LENGTH_PLACEHOLDER_SIZE];

        write_hexadecimal_to_buffer(&mut chank_length_str.clone(), chunk.len())?;
        self.base.new_line().await?;

        // Write chunk data
        self.base.extend_from_slice(chunk).await?;
        // Write CRLF after chunk
        self.base.new_line().await?;

        Ok(self)
    }

    /// Finalizes the chunked body by writing the zero-length chunk.
    pub async fn finalize_chunked_body(mut self) -> Result<HttpResponse, Error> {
        // Write zero-length chunk to indicate end of chunks
        self.base.extend_from_str("0\r\n\r\n").await?;
        self.base.flush().await?;
        Ok(HttpResponse::new())
    }
}

impl<'buffer, WriteSocket: SocketWrite> HttpResponseBuilder<'buffer, WriteSocket, stages::BuildChankedBodyWithTrailer> {
    /// Adds a chunk to the chunked body.
    pub async fn with_chunk(mut self, chunk: &[u8]) -> Result<Self, Error> {
        // Write chunk size in hexadecimal
        let chank_length_str = [b'0'; CHUNK_LENGTH_PLACEHOLDER_SIZE];

        write_hexadecimal_to_buffer(&mut chank_length_str.clone(), chunk.len())?;
        self.base.new_line().await?;

        // Write chunk data
        self.base.extend_from_slice(chunk).await?;
        // Write CRLF after chunk
        self.base.new_line().await?;

        Ok(self)
    }

    /// Finalizes the chunked body by writing the zero-length chunk.
    pub async fn finalize_chunked_body(
        mut self,
    ) -> Result<HttpResponseBuilder<'buffer, WriteSocket, stages::Trailer>, Error> {
        // Write zero-length chunk to indicate end of chunks
        self.base.extend_from_str("0\r\n\r\n").await?;
        Ok(HttpResponseBuilder {
            base: self.base,
            _phantom: core::marker::PhantomData,
        })
    }
}

impl<'buffer, WriteSocket: SocketWrite> HttpResponseBuilder<'buffer, WriteSocket, stages::Trailer> {
    /// Adds a trailer header to the HTTP response.
    pub async fn with_trailer_header(mut self, header: &str, value: &str) -> Result<Self, Error> {
        self.base.extend_from_str(header).await?;
        self.base.extend_from_str(": ").await?;
        self.base.extend_from_str(value).await?;
        self.base.new_line().await?;

        Ok(self)
    }

    /// Finalizes the chunked body by writing the zero-length chunk.
    pub async fn finalize_trailer(mut self) -> Result<HttpResponse, Error> {
        self.base.flush().await?;
        Ok(HttpResponse::new())
    }
}

impl<'buffer, WriteSocket: SocketWrite> BuilderBase<'buffer, WriteSocket> {
    #[inline]
    pub async fn new_line(&mut self) -> Result<(), Error> {
        self.write_socket
            .write_all(b"\r\n")
            .await
            .map_err(|_| Error::SocketError)
    }

    pub async fn extend_from_slice(&mut self, slice: &[u8]) -> Result<(), Error> {
        //TODO: return more precise error instead of mapping all write errors to SocketError
        self.write_socket.write_all(slice).await.map_err(|_| Error::SocketError)
    }

    #[inline]
    pub async fn extend_from_str(&mut self, s: &str) -> Result<(), Error> {
        self.extend_from_slice(s.as_bytes()).await
    }

    /// Flushes the write socket to ensure all data is sent.
    pub async fn flush(&mut self) -> Result<(), Error> {
        self.write_socket.flush().await.map_err(|_| Error::SocketError)
    }
}

/// Write a decimal number to the buffer
async fn extend_from_decimal<W: SocketWrite>(out_stream: &mut W, mut num: usize) -> Result<(), Error> {
    if num == 0 {
        //TODO: return more precise error instead of mapping all write errors to SocketError
        out_stream.write_all(b"0").await.map_err(|_| Error::SocketError)?;
        return Ok(());
    }

    let mut digits = [0u8; 10];
    let mut i = 0;

    while num > 0 {
        #[allow(clippy::cast_possible_truncation)]
        {
            digits[i] = (num % 10) as u8 + b'0';
        }
        num /= 10;
        i += 1;
    }

    // Write digits in reverse order
    for j in (0..i).rev() {
        //TODO: return more precise error instead of mapping all write errors to SocketError
        out_stream
            .write_all(&[digits[j]])
            .await
            .map_err(|_| Error::SocketError)?;
    }

    Ok(())
}

fn write_decimal_to_buffer(bytes: &mut [u8], mut num: usize) -> Result<(), Error> {
    if bytes.is_empty() {
        return Err(Error::MemoryOverflow);
    }
    bytes.fill(b'0');

    if num == 0 {
        return Ok(());
    }

    let mut i = bytes.len();

    while num > 0 {
        if i == 0 {
            // Not enough space to write the number
            return Err(Error::MemoryOverflow);
        }

        #[allow(clippy::cast_possible_truncation)]
        {
            bytes[i - 1] = (num % 10) as u8 + b'0';
        }
        num /= 10;
        i -= 1;
    }

    Ok(())
}

fn write_hexadecimal_to_buffer(bytes: &mut [u8], mut num: usize) -> Result<(), Error> {
    if bytes.is_empty() {
        return Err(Error::MemoryOverflow);
    }
    bytes.fill(b'0');

    if num == 0 {
        return Ok(());
    }

    let mut i = bytes.len();

    while num > 0 {
        if i == 0 {
            // Not enough space to write the number
            return Err(Error::MemoryOverflow);
        }

        let digit = (num % 16) as u8;
        bytes[i - 1] = if digit < 10 { digit + b'0' } else { digit - 10 + b'A' };
        num /= 16;
        i -= 1;
    }

    Ok(())
}

#[cfg(test)]
mod tests {

    // TODO: Add more tests for headers and body building
}
