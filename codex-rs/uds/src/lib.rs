use std::io::Result as IoResult;
use std::path::Path;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::io::ReadBuf;

pub async fn prepare_private_socket_directory(socket_dir: impl AsRef<Path>) -> IoResult<()> {
    platform::prepare_private_socket_directory(socket_dir.as_ref()).await
}

pub async fn is_stale_socket_path(socket_path: impl AsRef<Path>) -> IoResult<bool> {
    platform::is_stale_socket_path(socket_path.as_ref()).await
}

pub struct UnixListener {
    inner: platform::Listener,
}

impl UnixListener {
    pub async fn bind(socket_path: impl AsRef<Path>) -> IoResult<Self> {
        platform::bind_listener(socket_path.as_ref())
            .await
            .map(|inner| Self { inner })
    }

    pub async fn accept(&mut self) -> IoResult<UnixStream> {
        self.inner.accept().await.map(|inner| UnixStream { inner })
    }
}

pub struct UnixStream {
    inner: platform::Stream,
}

impl UnixStream {
    pub async fn connect(socket_path: impl AsRef<Path>) -> IoResult<Self> {
        platform::connect_stream(socket_path.as_ref())
            .await
            .map(|inner| Self { inner })
    }
}

impl AsyncRead for UnixStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<IoResult<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for UnixStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<IoResult<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<IoResult<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<IoResult<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

#[cfg(unix)]
mod platform {
    use std::io;
    use std::io::ErrorKind;
    use std::io::Result as IoResult;
    use std::os::unix::fs::FileTypeExt;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use tokio::fs;
    use tokio::net::UnixListener;
    use tokio::net::UnixStream;

    const SOCKET_DIR_MODE: u32 = 0o700;
    const SOCKET_DIR_PERMISSION_BITS: u32 = 0o777;

    pub(super) type Stream = UnixStream;

    pub(super) struct Listener(UnixListener);

    pub(super) async fn prepare_private_socket_directory(socket_dir: &Path) -> IoResult<()> {
        let mut dir_builder = fs::DirBuilder::new();
        dir_builder.mode(SOCKET_DIR_MODE);
        match dir_builder.create(socket_dir).await {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }

        let metadata = fs::symlink_metadata(socket_dir).await?;
        if !metadata.is_dir() {
            return Err(io::Error::new(
                ErrorKind::AlreadyExists,
                format!(
                    "socket directory path exists and is not a directory: {}",
                    socket_dir.display()
                ),
            ));
        }

        let permissions = metadata.permissions();

        if permissions.mode() & SOCKET_DIR_PERMISSION_BITS != SOCKET_DIR_MODE {
            fs::set_permissions(socket_dir, std::fs::Permissions::from_mode(SOCKET_DIR_MODE))
                .await?;
        }

        Ok(())
    }

    pub(super) async fn bind_listener(socket_path: &Path) -> IoResult<Listener> {
        UnixListener::bind(socket_path).map(Listener)
    }

    impl Listener {
        pub(super) async fn accept(&mut self) -> IoResult<Stream> {
            self.0.accept().await.map(|(stream, _addr)| stream)
        }
    }

    pub(super) async fn connect_stream(socket_path: &Path) -> IoResult<Stream> {
        UnixStream::connect(socket_path).await
    }

    pub(super) async fn is_stale_socket_path(socket_path: &Path) -> IoResult<bool> {
        Ok(fs::symlink_metadata(socket_path)
            .await?
            .file_type()
            .is_socket())
    }
}

#[cfg(test)]
mod lib_tests;
