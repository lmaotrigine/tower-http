use crate::services::fs::backend::{Backend, File, Metadata};
use futures_util::{future::BoxFuture, ready};
use std::{
    io::{self, SeekFrom},
    path::Path,
    pin::Pin,
    task::Poll,
    time::SystemTime,
};
use tokio::io::{AsyncRead, AsyncSeek};
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct IncludeDirBackend {
    inner: &'static include_dir::Dir<'static>,
}

impl IncludeDirBackend {
    pub const fn new(inner: &'static include_dir::Dir<'static>) -> Self {
        Self { inner }
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct IncludeDirFile {
    index: usize,
    inner: include_dir::File<'static>,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct IncludeDirMetadata {
    entry: include_dir::DirEntry<'static>,
}

impl Backend for IncludeDirBackend {
    type File = IncludeDirFile;
    type Metadata = IncludeDirMetadata;
    type OpenFuture = BoxFuture<'static, io::Result<Self::File>>;
    type MetadataFuture = BoxFuture<'static, io::Result<Self::Metadata>>;

    fn open<P: AsRef<Path>>(&self, path: P) -> Self::OpenFuture {
        let path = path.as_ref().to_owned();
        let this = self.clone();
        Box::pin(async move {
            Ok(IncludeDirFile {
                index: 0,
                inner: this
                    .inner
                    .get_file(&path)
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::NotFound,
                            format!("{} is not a file.", path.display()),
                        )
                    })?
                    .clone(),
            })
        })
    }

    fn metadata<P: AsRef<Path>>(&self, path: P) -> Self::MetadataFuture {
        let path = path.as_ref().to_owned();
        let this = self.clone();
        Box::pin(async move {
            this.inner
                .get_entry(&path)
                .map(|f| IncludeDirMetadata { entry: f.clone() })
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("{} not found.", path.display()),
                    )
                })
        })
    }
}

impl AsyncRead for IncludeDirFile {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let mut data = self.inner.contents();
        if self.index >= data.len() {
            return Poll::Ready(Ok(()));
        }
        data = &data[self.index..];
        let before_filled = buf.filled().len();
        ready!(Pin::new(&mut data).poll_read(cx, buf))?;
        let filled = buf.filled().len() - before_filled;
        self.index += filled;
        Poll::Ready(Ok(()))
    }
}

impl AsyncSeek for IncludeDirFile {
    fn start_seek(mut self: Pin<&mut Self>, position: SeekFrom) -> io::Result<()> {
        match position {
            SeekFrom::Start(start) => self.index = start as _,
            SeekFrom::End(end) => {
                self.index = self
                    .inner
                    .contents()
                    .len()
                    .checked_add_signed(end as _)
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "invalid seek to a negative or overflowing position",
                        )
                    })?;
            }
            SeekFrom::Current(index) => {
                self.index = self.index.checked_add_signed(index as _).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "invalid seek to a negative or overflowing position",
                    )
                })?;
            }
        }
        Ok(())
    }

    #[inline]
    fn poll_complete(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<io::Result<u64>> {
        Poll::Ready(Ok(self.index as _))
    }
}

impl File for IncludeDirFile {
    type Metadata = IncludeDirMetadata;
    type MetadataFuture<'a> = BoxFuture<'a, io::Result<Self::Metadata>>;

    fn metadata(&self) -> Self::MetadataFuture<'_> {
        let this = self.clone();
        Box::pin(async move {
            Ok(IncludeDirMetadata {
                entry: include_dir::DirEntry::File(this.inner),
            })
        })
    }
}

impl Metadata for IncludeDirMetadata {
    #[inline]
    fn is_dir(&self) -> bool {
        self.entry.as_dir().is_some()
    }

    #[inline]
    fn modified(&self) -> io::Result<SystemTime> {
        Ok(self
            .entry
            .as_file()
            .and_then(|f| f.metadata())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("cannot access metadata for {}", self.entry.path().display()),
                )
            })?
            .modified())
    }

    #[inline]
    fn len(&self) -> u64 {
        self.entry.as_file().map_or(0, |f| f.contents().len() as _)
    }
}
