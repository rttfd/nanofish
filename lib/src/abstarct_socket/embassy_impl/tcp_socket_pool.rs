use crate::abstarct_socket::socket::*;

use embassy_futures::select::*;
use embassy_net::tcp::TcpSocket;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;

use core::pin::Pin;
use defmt_or_log as log;
use pin_project::pin_project;

type Mutex<T> = embassy_sync::mutex::Mutex<NoopRawMutex, T>;
type MutexGuard<'a, T> = embassy_sync::mutex::MutexGuard<'a, NoopRawMutex, T>;
type Channel<T, const N: usize> = embassy_sync::channel::Channel<NoopRawMutex, T, N>;

/// A socket guard type that provides exclusive access to a TcpSocket for the duration of its lifetime.
/// The guard is created by the TcpSocketPool and ensures that the socket is not accessed concurrently
/// by multiple tasks.
type SocketGuard<'a> = MutexGuard<'a, TcpSocket<'a>>;

type Socket<'a> = Mutex<TcpSocket<'a>>;
type SocketQueue<'a, const N: usize> = Channel<SocketGuard<'a>, N>;

const KEEP_ALIVE_TIMEOUT: embassy_time::Duration = embassy_time::Duration::from_secs(3);
const SOCKET_IO_TIMEOUT: embassy_time::Duration = embassy_time::Duration::from_secs(5);

/// A socket pool implementation for managing multiple TCP socket connections using embassy-net.
pub struct TcpSocketPool<'pool, const SOCKETS: usize> {
    state: Pin<&'pool TcpSocketPoolState<'pool, SOCKETS>>,
}

impl<'pool, const SOCKETS: usize> TcpSocketPool<'pool, SOCKETS> {
    /// Create a new TcpSocketPool with the given stack, buffer, and endpoint.
    pub fn new(
        state: Pin<&'pool mut TcpSocketPoolState<'pool, SOCKETS>>,
    ) -> (Self, TcpSocketPoolRunner<'pool, SOCKETS>) {
        let state = state.into_ref();

        (Self { state }, TcpSocketPoolRunner { state })
    }
}

impl<'pool, const SOCKETS: usize> AbstractSocketListener for TcpSocketPool<'pool, SOCKETS> {
    type Socket = PoolSocket<'pool>;

    async fn accept(&self) -> Self::Socket {
        unsafe { core::mem::transmute::<_, Self::Socket>(PoolSocket::new(self.state.queue.receive().await)) }
    }

    async fn try_accept(&self) -> Option<Self::Socket> {
        unsafe {
            core::mem::transmute::<_, Option<Self::Socket>>(self.state.queue.try_receive().map(PoolSocket::new).ok())
        }
    }

    fn local_endpoint(&self) -> SocketEndpoint {
        self.state.endpoint
    }
}

/// Internal state of the TcpSocketPool, which manages the actual sockets and the accept loop.
pub struct TcpSocketPoolRunner<'pool, const SOCKETS: usize> {
    state: Pin<&'pool TcpSocketPoolState<'pool, SOCKETS>>,
}

impl<'pool, const SOCKETS: usize> TcpSocketPoolRunner<'pool, SOCKETS> {
    /// Run the accept loop for the TcpSocketPool, which continuously accepts incoming connections and pushes ready sockets into the queue.
    pub async fn run(self) -> ! {
        self.state.accept_all_loop().await;
    }
}

/// The state of the TcpSocketPool, which contains the sockets, the queue for ready sockets, and the endpoint information.
#[pin_project]
pub struct TcpSocketPoolState<'stack, const SOCKETS: usize> {
    #[pin]
    sockets: [Socket<'stack>; SOCKETS],
    queue: SocketQueue<'static, SOCKETS>,
    endpoint: SocketEndpoint,
}

impl<'stack, const SOCKETS: usize> TcpSocketPoolState<'stack, SOCKETS> {
    /// Create a new TcpSocketPoolState with the given stack, buffer, and endpoint.
    pub fn new<const RX_SIZE: usize, const TX_SIZE: usize>(
        stack: embassy_net::Stack<'stack>,
        buffer: &'stack mut [u8],
        endpoint: SocketEndpoint,
    ) -> Self {
        const {
            assert!(SOCKETS > 0, "Socket pool size must be greater than zero");
        };

        log::assert!(
            buffer.len() >= SOCKETS * (RX_SIZE + TX_SIZE),
            "SocketPool: Buffer size must be at least SOCKETS * (RX_SIZE + TX_SIZE)"
        );

        let mut chunks = buffer.chunks_exact_mut(RX_SIZE + TX_SIZE).map(|mem_chunk| {
            let (rx_buffer, tx_buffer) = mem_chunk.split_at_mut(RX_SIZE);
            (rx_buffer, tx_buffer)
        });

        let sockets = core::array::from_fn::<_, SOCKETS, _>(|_| {
            // SAFETY: We have already checked that the buffer has enough space for SOCKETS pairs of RX and TX buffers, so this unwrap is safe.
            let (rx_buffer, tx_buffer) = unsafe { chunks.next().unwrap_unchecked() };
            let mut socket = TcpSocket::new(stack, rx_buffer, tx_buffer);

            // Set keep alive options (This must be set to prevent connections from being closed by NATs)
            socket.set_keep_alive(Some(KEEP_ALIVE_TIMEOUT));
            // This must be set to prevent eternal pending on IO operations
            socket.set_timeout(Some(SOCKET_IO_TIMEOUT));

            // We guarantee that this object
            Mutex::new(socket)
        });
        Self {
            sockets,
            queue: SocketQueue::new(),
            endpoint,
        }
    }

    #[inline]
    async fn accept_all_loop(self: Pin<&TcpSocketPoolState<'stack, SOCKETS>>) -> ! {
        loop {
            select_array(core::array::from_fn::<_, SOCKETS, _>(|idx| self.accept_loop(idx))).await;
        }
    }

    async fn accept_loop(self: Pin<&TcpSocketPoolState<'stack, SOCKETS>>, index: usize) -> ! {
        use embassy_net::tcp::State;
        let this = self.project_ref();

        let accept = async move |socket: &'_ mut embassy_net::tcp::TcpSocket<'_>| {
            socket.accept(this.endpoint.port()).await.unwrap_or_else(|e| {
                log::panic!(
                    "SocketPool: Error while accepting connection[{}]: {:?}",
                    index,
                    log::Debug2Format(&e)
                );
            });
        };

        loop {
            let mut socket = this.sockets[index].lock().await;
            log::info!(
                "SocketPool:1: Socket[{}] released with state: {:?}",
                index,
                socket.state()
            );

            match socket.state() {
                State::Established | State::SynSent | State::SynReceived => {
                    if SocketWaitReadReady::wait_read_ready(&mut *socket).await.is_err() {
                        socket.close();
                        socket.flush().await.ok();
                        continue;
                    }
                }
                State::Closed | State::Listen => {
                    accept(&mut socket).await;
                    if SocketWaitReadReady::wait_read_ready(&mut *socket).await.is_err() {
                        socket.close();
                        socket.flush().await.ok();
                        continue;
                    }
                }
                State::TimeWait | State::FinWait1 | State::Closing | State::LastAck | State::CloseWait => {
                    // The socket is in a state where it is either closing or waiting for the remote to close,
                    // we need to gracefully bring it down first before accepting a new connection on it.
                    // Close the write side of the connection
                    socket.close();
                    // Ensure all pending data is sent
                    socket.flush().await.ok();
                    accept(&mut socket).await;
                    if SocketWaitReadReady::wait_read_ready(&mut *socket).await.is_err() {
                        socket.close();
                        continue;
                    }
                }
                State::FinWait2 => {
                    // The socket is waiting for the remote to close, we need to wait for it to be writable before accepting a new connection on it.
                    SocketWaitWriteReady::wait_write_ready(&mut *socket).await.ok();
                    socket.close();
                    continue;
                }
            };

            // Push ready socket into the queue
            // SAFETY: The socket is guaranteed to be valid for whole lifetime of the TcpSocketPoolData, and the SocketGuard will ensure that it is not accessed concurrently.
            let socket = unsafe { core::mem::transmute::<_, SocketGuard<'static>>(socket) };
            this.queue.send(socket).await;
            log::info!("SocketPool:2: Socket[{}] enqueued", index);
        }
    }
}

/// Represents a socket that is managed by the TcpSocketPool.
/// This type is used as the associated Socket type in the AbstractSocketListener implementation for
/// TcpSocketPool, and it provides access to the underlying TcpSocket through the SocketGuard.
pub struct PoolSocket<'pool>(SocketGuard<'pool>);

impl<'pool> PoolSocket<'pool> {
    /// Create a new PoolSocket from a SocketGuard.
    const fn new(guard: SocketGuard<'pool>) -> Self {
        Self(guard)
    }
}

impl SocketErrorType for PoolSocket<'_> {
    type Error = embassy_net::tcp::Error;
}

impl SocketReadWith for PoolSocket<'_> {
    #[inline]
    fn read_with<F, R>(&mut self, f: F) -> impl core::future::Future<Output = Result<R, Self::Error>>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        self.0.read_with::<F, R>(f)
    }
}

impl SocketWriteWith for PoolSocket<'_> {
    #[inline]
    fn write_with<F, R>(&mut self, f: F) -> impl core::future::Future<Output = Result<R, Self::Error>>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        self.0.write_with::<F, R>(f)
    }
}

impl SocketRead for PoolSocket<'_> {
    #[inline]
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(buf).await
    }

    #[inline]
    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), SocketReadExactError<Self::Error>> {
        self.0.read_exact(buf).await
    }
}

impl SocketWrite for PoolSocket<'_> {
    #[inline]
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(buf).await
    }

    #[inline]
    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await
    }

    #[inline]
    async fn write_all(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        self.0.write_all(buf).await
    }
}

impl SocketWriteReady for PoolSocket<'_> {
    #[inline]
    fn write_ready(&mut self) -> Result<bool, Self::Error> {
        self.0.write_ready()
    }
}

impl SocketReadReady for PoolSocket<'_> {
    #[inline]
    fn read_ready(&mut self) -> Result<bool, Self::Error> {
        self.0.read_ready()
    }
}

impl SocketWaitReadReady for PoolSocket<'_> {
    #[inline]
    async fn wait_read_ready(&mut self) -> Result<(), Self::Error> {
        <TcpSocket<'_> as SocketWaitReadReady>::wait_read_ready(&mut self.0).await
    }
}

impl SocketWaitWriteReady for PoolSocket<'_> {
    #[inline]
    async fn wait_write_ready(&mut self) -> Result<(), Self::Error> {
        <TcpSocket<'_> as SocketWaitWriteReady>::wait_write_ready(&mut self.0).await
    }
}

impl SocketClose for PoolSocket<'_> {
    type Error = embassy_net::tcp::Error;

    #[inline]
    async fn close(&mut self) -> Result<(), Self::Error> {
        <TcpSocket<'_> as SocketClose>::close(&mut self.0).await
    }
}

impl SocketInfo for PoolSocket<'_> {
    #[inline]
    fn local_endpoint(&self) -> Option<SocketEndpoint> {
        <TcpSocket<'_> as SocketInfo>::local_endpoint(&self.0)
    }

    #[inline]
    fn remote_endpoint(&self) -> Option<SocketEndpoint> {
        <TcpSocket<'_> as SocketInfo>::remote_endpoint(&self.0)
    }

    #[inline]
    fn state(&self) -> State {
        State::from(<TcpSocket<'_> as SocketInfo>::state(&self.0))
    }
}
