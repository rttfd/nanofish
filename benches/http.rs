//! Benchmarks for the hot paths: parsing a response, serving a request, and
//! the encoders that run on every request.

#![expect(
    clippy::expect_used,
    reason = "a benchmark with broken setup should stop loudly, like a test"
)]

use core::convert::Infallible;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use embedded_io_async::{ErrorType, Read, Write};
use futures_lite::future::block_on;
use nanofish::client::SliceSource;
use nanofish::encode::{base64_encode, percent_encode};
use nanofish::{
    Error, HttpClient, HttpClientRequest, HttpHandler, HttpMethod, HttpRequest, RequestBody,
    ServerResponse, SimpleHandler, StatusCode, handle_http_connection_with_sizes,
};
use std::hint::black_box;

/// Serves `input` at most `max` bytes per read and discards what is written.
struct Replay<'a> {
    input: &'a [u8],
    max: usize,
}

impl ErrorType for Replay<'_> {
    type Error = Infallible;
}

impl Read for Replay<'_> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let n = buf.len().min(self.max).min(self.input.len());
        buf[..n].copy_from_slice(&self.input[..n]);
        self.input = &self.input[n..];
        Ok(n)
    }
}

impl Write for Replay<'_> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        Ok(buf.len())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn length_response(body: usize) -> Vec<u8> {
    let mut raw =
        format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {body}\r\n\r\n")
            .into_bytes();
    raw.resize(raw.len() + body, b'x');
    raw
}

fn chunked_response(chunks: usize, chunk: usize) -> Vec<u8> {
    let mut raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    for _ in 0..chunks {
        raw.extend(format!("{chunk:x}\r\n").bytes());
        raw.resize(raw.len() + chunk, b'x');
        raw.extend(b"\r\n");
    }
    raw.extend(b"0\r\n\r\n");
    raw
}

fn client_response(c: &mut Criterion) {
    let client = HttpClient::<1024>::new();
    let request = || HttpClientRequest {
        method: HttpMethod::GET,
        host: "example.com",
        path: "/data",
        query: None,
        headers: &[],
        body: None,
    };
    let mut group = c.benchmark_group("client_response");
    for (name, raw) in [
        ("length_4k", length_response(4096)),
        ("chunked_16x256", chunked_response(16, 256)),
    ] {
        // Small reads are what makes rescanning a growing buffer expensive.
        for max in [64, 1460] {
            group.bench_with_input(BenchmarkId::new(name, max), &raw, |b, raw| {
                let mut buffer = vec![0; 8192];
                b.iter(|| {
                    let mut stream = Replay { input: raw, max };
                    let (response, used) =
                        block_on(client.request(&mut stream, request(), &mut buffer))
                            .expect("canned response parses");
                    black_box((response.status_code, used));
                });
            });
        }
    }
    group.finish();
}

/// Serves a canned 204 response to an upload: writes are discarded, the read
/// side replays the response.
fn client_upload(c: &mut Criterion) {
    let client = HttpClient::<1024>::new();
    let request = || HttpClientRequest {
        method: HttpMethod::PUT,
        host: "example.com",
        path: "/upload",
        query: None,
        headers: &[],
        body: None,
    };
    let body = vec![b'x'; 4096];
    let reply = b"HTTP/1.1 204 No Content\r\n\r\n";
    let mut group = c.benchmark_group("client_upload");
    // The chunk size drives how often the size line is formatted and written.
    for chunk in [64, 256, 1460] {
        group.bench_with_input(
            BenchmarkId::new("chunked_4k", chunk),
            &chunk,
            |b, &chunk| {
                let mut buffer = [0u8; 128];
                b.iter(|| {
                    let mut stream = Replay {
                        input: reply,
                        max: 1460,
                    };
                    let mut source = SliceSource::new(black_box(&body), chunk);
                    let (response, used) = block_on(client.request_upload(
                        &mut stream,
                        request(),
                        &mut source,
                        &mut buffer,
                    ))
                    .expect("canned response parses");
                    black_box((response.status_code, used));
                });
            },
        );
    }
    group.finish();
}

fn server_request(c: &mut Criterion) {
    let raw =
        b"GET /health HTTP/1.1\r\nHost: device.local\r\nUser-Agent: bench\r\nAccept: */*\r\n\r\n";
    c.bench_function("server_request/get_health", |b| {
        b.iter(|| {
            let mut stream = Replay {
                input: raw,
                max: 1460,
            };
            block_on(handle_http_connection_with_sizes::<_, _, 1024, 1024>(
                &mut stream,
                &mut SimpleHandler,
            ))
            .expect("request is served");
        });
    });
}

/// How a bench handler takes the request body.
#[derive(Clone, Copy)]
enum Take {
    /// Collect all of it with `RequestBody::all`.
    All,
    /// Read it piece by piece with `RequestBody::chunk`.
    Pieces,
}

/// Reads the request body the way `take` says and answers `200`.
struct BodyHandler {
    take: Take,
    seen: usize,
}

impl HttpHandler for BodyHandler {
    async fn handle_request<S: Read + Write>(
        &mut self,
        _request: &HttpRequest<'_>,
        body: &mut RequestBody<'_, S>,
    ) -> Result<ServerResponse<'_>, Error> {
        self.seen = 0;
        match self.take {
            Take::All => self.seen = body.all().await?.len(),
            Take::Pieces => {
                while let Some(piece) = body.chunk().await? {
                    self.seen += piece.len();
                }
            }
        }
        Ok(ServerResponse {
            status_code: StatusCode::Ok,
            headers: heapless::Vec::new(),
            body: b"ok",
        })
    }
}

fn length_request(body: usize) -> Vec<u8> {
    let mut raw =
        format!("POST /up HTTP/1.1\r\nHost: device.local\r\nContent-Length: {body}\r\n\r\n")
            .into_bytes();
    raw.resize(raw.len() + body, b'x');
    raw
}

fn chunked_request(chunks: usize, chunk: usize) -> Vec<u8> {
    let mut raw =
        b"POST /up HTTP/1.1\r\nHost: device.local\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    for _ in 0..chunks {
        raw.extend(format!("{chunk:x}\r\n").bytes());
        raw.resize(raw.len() + chunk, b'x');
        raw.extend(b"\r\n");
    }
    raw.extend(b"0\r\n\r\n");
    raw
}

/// Serves one request to a `BodyHandler` over a 1024-byte connection buffer
/// and returns how many body bytes the handler saw.
fn serve_body(raw: &[u8], take: Take) -> usize {
    let mut stream = Replay {
        input: raw,
        max: 1460,
    };
    let mut handler = BodyHandler { take, seen: 0 };
    block_on(handle_http_connection_with_sizes::<_, _, 1024, 1024>(
        &mut stream,
        &mut handler,
    ))
    .expect("request is served");
    handler.seen
}

fn server_body(c: &mut Criterion) {
    // A body that fits the buffer: `all()` is the cost of the old buffered
    // path, so it must stay cheap.
    let small = length_request(256);
    c.bench_function("server_body/all_256", |b| {
        b.iter(|| black_box(serve_body(black_box(&small), Take::All)));
    });
    // A handler that ignores the body: the server drains it after answering.
    c.bench_function("server_body/ignored_256", |b| {
        b.iter(|| {
            let mut stream = Replay {
                input: black_box(&small),
                max: 1460,
            };
            block_on(handle_http_connection_with_sizes::<_, _, 1024, 1024>(
                &mut stream,
                &mut SimpleHandler,
            ))
            .expect("request is served");
        });
    });

    // Bodies far larger than the buffer, which only streaming can take.
    let mut group = c.benchmark_group("server_body/stream_64k");
    for chunk in [256usize, 1460, 4096] {
        let raw = chunked_request(65_536 / chunk, chunk);
        group.throughput(criterion::Throughput::Bytes(65_536));
        group.bench_with_input(BenchmarkId::new("chunked", chunk), &raw, |b, raw| {
            b.iter(|| black_box(serve_body(black_box(raw), Take::Pieces)));
        });
    }
    let raw = length_request(65_536);
    group.throughput(criterion::Throughput::Bytes(65_536));
    group.bench_with_input(BenchmarkId::new("length", 65_536), &raw, |b, raw| {
        b.iter(|| black_box(serve_body(black_box(raw), Take::Pieces)));
    });
    group.finish();
}

fn encoders(c: &mut Criterion) {
    let mut out = [0u8; 256];
    c.bench_function("encode/percent", |b| {
        b.iter(|| {
            black_box(
                percent_encode(black_box("name=A B&path=/led/3?fade=1"), &mut out)
                    .expect("fits")
                    .len(),
            )
        });
    });
    c.bench_function("encode/base64", |b| {
        b.iter(|| {
            black_box(
                base64_encode(black_box(b"device-user:correct horse battery"), &mut out)
                    .expect("fits")
                    .len(),
            )
        });
    });
}

criterion_group!(
    benches,
    client_response,
    client_upload,
    server_request,
    server_body,
    encoders
);
criterion_main!(benches);
