#![doc = include_str!("../README.md")]

use nanooctopus::*;

/// Request handler that responds to every HTTP request with "Hello, World!".
struct HelloWorldHandler;

impl http_handler::HttpHandler for HelloWorldHandler {
    async fn handle_request(
        &mut self,
        _allocator: &mut http_handler::HttpAllocator<'_>, // unused in this simple handler
        request: &http_handler::HttpRequest<'_>,
        http_socket: &mut impl http_handler::HttpSocketWrite,
        context_id: usize,
    ) -> Result<http_handler::HttpResponse, http_handler::Error> {
        log::info!("HelloWorldHandler[{}]: Received request: {:?}", context_id, request);

        // Stream the response directly to the socket: status → headers → body.
        http_handler::HttpResponseBuilder::new(http_socket)
            .with_status(http_handler::StatusCode::Ok)
            .await?
            .with_header("Content-Type", "text/plain")
            .await?
            .with_body_from_slice(b"Hello, World!")
            .await
    }
}

fn init_logging() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).try_init();
}

#[tokio::main(flavor = "local")]
async fn main() {
    init_logging();

    // `spawn_local` keeps everything on the current thread, matching the
    // single-threaded (`flavor = "local"`) Tokio runtime used here.
    tokio::task::spawn_local(async move {
        // Bind the TCP listener to localhost:8080.
        let listener =
            server::socket_listener::TokioTcpListener::new(server::SocketEndpoint::new([127, 0, 0, 1].into(), 8080))
                .await;

        let server = server::HttpServer::new(listener, server::ServerTimeouts::default());

        // 1024-byte scratch buffer for parsing incoming HTTP headers.
        // A single worker (context_id = 1) handles requests sequentially.
        server
            .serve(server::HttpWorkerMemory::<1024>::new(), HelloWorldHandler, 1)
            .await
    })
    .await
    .unwrap();
}
