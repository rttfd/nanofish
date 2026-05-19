extern crate alloc;
extern crate std;

#[cfg(test)]
mod tests {
    use core::net::*;

    use embedded_io_async::Read;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    use nanooctopus::socket::tokio_impl::socket::{
        TokioSocketReadHalfWrapper, TokioTcpListener, TokioTcpSocketConnector,
    };
    use nanooctopus::socket::{
        AbstractSocketConnector, AbstractSocketListener, SocketClose, SocketInfo, SocketReadWith, State,
    };

    #[tokio::test]
    async fn test_stream_read_with_preserves_unconsumed_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = listener.local_addr().unwrap();

        let writer = tokio::spawn(async move {
            let (mut peer, _) = listener.accept().await.unwrap();
            peer.write_all(b"abcdef").await.unwrap();
        });

        let mut socket = TokioTcpSocketConnector::new().connect(endpoint).await.unwrap();

        let prefix = socket
            .read_with(|buf| {
                assert_eq!(&buf[..6], b"abcdef");
                (2, [buf[0], buf[1]])
            })
            .await
            .unwrap();
        assert_eq!(prefix, *b"ab");

        let mut tail = [0u8; 8];
        let read = socket.read(&mut tail).await.unwrap();
        assert_eq!(&tail[..read], b"cdef");

        writer.await.unwrap();
    }

    #[tokio::test]
    async fn test_read_half_read_with_preserves_unconsumed_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = listener.local_addr().unwrap();

        let writer = tokio::spawn(async move {
            let (mut peer, _) = listener.accept().await.unwrap();
            peer.write_all(b"abcdef").await.unwrap();
        });

        let mut stream = TcpStream::connect(endpoint).await.unwrap();
        let (read_half, _) = stream.split();
        let mut socket = TokioSocketReadHalfWrapper::new(read_half);

        let prefix = socket
            .read_with(|buf| {
                assert_eq!(&buf[..6], b"abcdef");
                (2, [buf[0], buf[1]])
            })
            .await
            .unwrap();
        assert_eq!(prefix, *b"ab");

        let mut tail = [0u8; 8];
        let read = socket.read(&mut tail).await.unwrap();
        assert_eq!(&tail[..read], b"cdef");

        writer.await.unwrap();
    }

    #[tokio::test]
    async fn test_connect_promotes_socket_to_stream_and_sets_endpoints() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_endpoint = listener.local_addr().unwrap();

        let accept_task = tokio::spawn(async move {
            let (stream, remote_addr) = listener.accept().await.unwrap();
            (stream.local_addr().unwrap(), remote_addr)
        });

        let socket = TokioTcpSocketConnector::new().connect(server_endpoint).await.unwrap();

        let client_local = socket.local_endpoint().unwrap();
        let client_remote = socket.remote_endpoint().unwrap();
        assert_eq!(client_remote, server_endpoint);

        let (server_local, server_remote) = accept_task.await.unwrap();
        assert_eq!(client_local, server_remote);
        assert_eq!(server_local, server_endpoint);
    }

    #[tokio::test]
    async fn test_close_makes_peer_observe_eof() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_endpoint = listener.local_addr().unwrap();

        let peer_task = tokio::spawn(async move {
            let (mut peer_stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 8];
            peer_stream.read(&mut buf).await.unwrap()
        });

        let mut socket = TokioTcpSocketConnector::new().connect(server_endpoint).await.unwrap();
        socket.close().await.unwrap();

        assert_eq!(peer_task.await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_listener_exposes_local_endpoint_and_accepts_connection() {
        let listener = TokioTcpListener::new("127.0.0.1:0".parse::<SocketAddr>().unwrap()).await;
        let listen_endpoint = listener.local_endpoint();
        let client = tokio::spawn(async move { TcpStream::connect(listen_endpoint).await.unwrap() });

        let socket = listener.accept().await;
        assert!(
            socket.state() == State::Established,
            "Socket should be in Established state after accept"
        );

        let client = client.await.unwrap();

        assert_eq!(socket.local_endpoint(), Some(listen_endpoint));
        assert_eq!(socket.remote_endpoint(), Some(client.local_addr().unwrap()));
    }

    mod tokio_listener {
        use super::*;
        use nanooctopus::socket::AbstractSocketListener;
        use nanooctopus::socket::tokio_impl::socket::TokioTcpListener;
        use tokio::net::TcpStream;

        #[tokio::test]
        async fn builder_creates_socket() {
            let listener = TokioTcpListener::new("127.0.0.1:0".parse::<SocketAddr>().unwrap()).await;
            let listen_endpoint = listener.local_endpoint();

            let client = tokio::spawn(async move { TcpStream::connect(listen_endpoint).await.unwrap() });

            let _wrapper = listener.accept().await;
            client.await.unwrap();
        }
    }
}
