use crate::socket::{
    SocketClose, SocketEndpoint, SocketInfo, SocketReadWith, SocketWaitReadReady, SocketWaitWriteReady,
    SocketWriteWith, State,
};

use embassy_net::tcp::{TcpReader, TcpSocket, TcpWriter};
use embedded_io_async::ReadReady;

// Embassy-net based ReadStream implementation for TcpReader
impl<'stack> SocketReadWith for TcpSocket<'stack> {
    #[inline]
    fn read_with<F, R>(&mut self, f: F) -> impl core::future::Future<Output = Result<R, Self::Error>>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        self.read_with(f)
    }
}

// Embassy-net based implementation of ReadStream for TcpReader
impl<'stack> SocketReadWith for TcpReader<'stack> {
    #[inline]
    fn read_with<F, R>(&mut self, f: F) -> impl core::future::Future<Output = Result<R, Self::Error>>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        self.read_with(f)
    }
}

// Embassy-net based WriteWith implementation for TcpSocket
impl<'stack> SocketWriteWith for TcpSocket<'stack> {
    #[inline]
    fn write_with<F, R>(&mut self, f: F) -> impl core::future::Future<Output = Result<R, Self::Error>>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        self.write_with(f)
    }
}

// Embassy-net based implementation of WriteWith for TcpWriter
impl<'stack> SocketWriteWith for TcpWriter<'stack> {
    #[inline]
    fn write_with<F, R>(&mut self, f: F) -> impl core::future::Future<Output = Result<R, Self::Error>>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        self.write_with(f)
    }
}

impl From<embassy_net::tcp::State> for State {
    fn from(state: embassy_net::tcp::State) -> Self {
        match state {
            embassy_net::tcp::State::Closed => State::Closed,
            embassy_net::tcp::State::Listen => State::Listen,
            embassy_net::tcp::State::SynSent => State::SynSent,
            embassy_net::tcp::State::SynReceived => State::SynReceived,
            embassy_net::tcp::State::Established => State::Established,
            embassy_net::tcp::State::FinWait1 => State::FinWait1,
            embassy_net::tcp::State::FinWait2 => State::FinWait2,
            embassy_net::tcp::State::CloseWait => State::CloseWait,
            embassy_net::tcp::State::Closing => State::Closing,
            embassy_net::tcp::State::LastAck => State::LastAck,
            embassy_net::tcp::State::TimeWait => State::TimeWait,
        }
    }
}

impl From<State> for embassy_net::tcp::State {
    fn from(state: State) -> Self {
        match state {
            State::Closed => embassy_net::tcp::State::Closed,
            State::Listen => embassy_net::tcp::State::Listen,
            State::SynSent => embassy_net::tcp::State::SynSent,
            State::SynReceived => embassy_net::tcp::State::SynReceived,
            State::Established => embassy_net::tcp::State::Established,
            State::FinWait1 => embassy_net::tcp::State::FinWait1,
            State::FinWait2 => embassy_net::tcp::State::FinWait2,
            State::CloseWait => embassy_net::tcp::State::CloseWait,
            State::Closing => embassy_net::tcp::State::Closing,
            State::LastAck => embassy_net::tcp::State::LastAck,
            State::TimeWait => embassy_net::tcp::State::TimeWait,
        }
    }
}

fn from_embassy_endpoint(endpoint: embassy_net::IpEndpoint) -> SocketEndpoint {
    match endpoint.addr {
        embassy_net::IpAddress::Ipv4(addr) => SocketEndpoint::V4(core::net::SocketAddrV4::new(addr, endpoint.port)),
        #[cfg(feature = "proto-ipv6")]
        embassy_net::IpAddress::Ipv6(addr) => {
            SocketEndpoint::V6(core::net::SocketAddrV6::new(addr, endpoint.port, 0, 0))
        }
    }
}

impl SocketInfo for TcpSocket<'_> {
    #[inline]
    fn local_endpoint(&self) -> Option<SocketEndpoint> {
        self.local_endpoint().map(from_embassy_endpoint)
    }

    #[inline]
    fn remote_endpoint(&self) -> Option<SocketEndpoint> {
        self.remote_endpoint().map(from_embassy_endpoint)
    }

    #[inline]
    fn state(&self) -> State {
        State::from(self.state())
    }
}

impl SocketClose for TcpSocket<'_> {
    type Error = embassy_net::tcp::Error;
    #[inline]
    async fn close(&mut self) -> Result<(), Self::Error> {
        // Close the write side of the connection
        self.close();
        // Ensure all pending data is sent
        self.flush().await?;
        // Close the socket
        self.abort();
        // Ensure the RST is sent
        self.flush().await?;
        Result::<_, Self::Error>::Ok(())
    }
}

impl SocketWaitReadReady for TcpSocket<'_> {
    async fn wait_read_ready(&mut self) -> Result<(), Self::Error> {
        use core::future::poll_fn;
        use core::pin::pin;
        use core::task::Poll;

        poll_fn(|cx| -> Poll<Result<(), Self::Error>> {
            let wait_read_ready = pin!(TcpSocket::wait_read_ready(self));
            if let Poll::Ready(()) = wait_read_ready.poll(cx) {
                Poll::Ready(Ok(()))
            } else {
                if !self.may_recv() {
                    // If the socket is not ready for reading and cannot receive more data, it means the connection has been closed.
                    return Poll::Ready(Err(Self::Error::ConnectionReset));
                }
                Poll::Pending
            }
        })
        .await
    }
}

impl SocketWaitReadReady for TcpReader<'_> {
    async fn wait_read_ready(&mut self) -> Result<(), Self::Error> {
        use core::future::poll_fn;
        use core::pin::pin;
        use core::task::Poll;

        poll_fn(|cx| -> Poll<Result<(), Self::Error>> {
            if let Poll::Ready(()) = pin!(TcpReader::wait_read_ready(self)).poll(cx) {
                Poll::Ready(Ok(()))
            } else {
                if !self.read_ready().unwrap() {
                    // If the socket is not ready for reading and cannot receive more data, it means the connection has been closed.
                    return Poll::Ready(Err(Self::Error::ConnectionReset));
                }
                Poll::Pending
            }
        })
        .await
    }
}

impl SocketWaitWriteReady for TcpSocket<'_> {
    async fn wait_write_ready(&mut self) -> Result<(), Self::Error> {
        use core::future::poll_fn;
        use core::pin::pin;
        use core::task::Poll;

        poll_fn(|cx| -> Poll<Result<(), Self::Error>> {
            if let Poll::Ready(()) = pin!(TcpSocket::wait_write_ready(self)).poll(cx) {
                Poll::Ready(Ok(()))
            } else {
                if let Poll::Ready(Err(e)) = pin!(TcpSocket::flush(self)).poll(cx) {
                    // If flushing the socket results in an error, it means the connection has been closed.
                    return Poll::Ready(Err(e));
                }
                Poll::Pending
            }
        })
        .await
    }
}

impl SocketWaitWriteReady for TcpWriter<'_> {
    async fn wait_write_ready(&mut self) -> Result<(), Self::Error> {
        use core::future::poll_fn;
        use core::pin::pin;
        use core::task::Poll;

        poll_fn(|cx| -> Poll<Result<(), Self::Error>> {
            if let Poll::Ready(()) = pin!(TcpWriter::wait_write_ready(self)).poll(cx) {
                Poll::Ready(Ok(()))
            } else {
                if let Poll::Ready(Err(e)) = pin!(TcpWriter::flush(self)).poll(cx) {
                    // If flushing the socket results in an error, it means the connection has been closed.
                    return Poll::Ready(Err(e));
                }
                Poll::Pending
            }
        })
        .await
    }
}
