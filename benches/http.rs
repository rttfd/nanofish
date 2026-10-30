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
use nanofish::encode::{base64_encode, percent_encode};
use nanofish::{
    HttpClient, HttpClientRequest, HttpMethod, SimpleHandler, handle_http_connection_with_sizes,
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

criterion_group!(benches, client_response, server_request, encoders);
criterion_main!(benches);
