use crate::socket::WriteWith;
use tokio::io::AsyncWriteExt;

use crate::tokio_impl::tokio_socket_wrapper::{
    TokioSocketOwnedWriteHalfWrapper, TokioSocketWrapper, TokioSocketWriteHalfWrapper,
};

const WRITE_BUFFER_SIZE: usize = 1024;

impl WriteWith for TokioSocketWrapper {
    async fn write_with<F, R>(&mut self, f: F) -> Result<R, Self::Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        match self {
            TokioSocketWrapper::Stream(stream) => {
                stream.writable().await?;
                let mut buf = [0u8; WRITE_BUFFER_SIZE];
                let (written, result) = f(&mut buf);
                assert!(written <= buf.len(), "Wrote more bytes than available in buffer");
                stream.write_all(&buf[..written]).await?;
                Ok(result)
            }
            TokioSocketWrapper::Socket(_) => {
                Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Tokio socket is not connected").into())
            }
            TokioSocketWrapper::Listener(_) => {
                Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Tokio listener cannot be written to").into())
            }
        }
    }
}

impl<'socket> WriteWith for TokioSocketWriteHalfWrapper<'socket> {
    async fn write_with<F, R>(&mut self, f: F) -> Result<R, Self::Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        let mut buf = [0u8; WRITE_BUFFER_SIZE];
        let (written, result) = f(&mut buf);
        assert!(written <= buf.len(), "Wrote more bytes than available in buffer");
        self.inner_mut().write_all(&buf[..written]).await?;
        Ok(result)
    }
}

impl WriteWith for TokioSocketOwnedWriteHalfWrapper {
    async fn write_with<F, R>(&mut self, f: F) -> Result<R, Self::Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        let mut buf = [0u8; WRITE_BUFFER_SIZE];
        let (written, result) = f(&mut buf);
        assert!(written <= buf.len(), "Wrote more bytes than available in buffer");
        self.inner_mut().write_all(&buf[..written]).await?;
        Ok(result)
    }
}
