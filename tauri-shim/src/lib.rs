//! Just enough Tauri for `commands.rs` to compile and run unchanged:
//! `#[command]` (a no-op), `State`, `ipc::Channel` (a callback),
//! `ipc::Response` (bytes), `AppHandle` + `Manager` (the cache folder Tauri
//! would use, so the two apps share thumbnails), and
//! `async_runtime::spawn_blocking` (an OS thread).
//!
//! The native app calls a command directly; an `async` one is driven with
//! [`block_on`] on a worker thread.

use std::ops::Deref;
use std::path::PathBuf;

pub use tauri_shim_macros::command;

/// Drive a command's future to the end on this thread.
pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
    pollster::block_on(f)
}

/// Managed state, borrowed for the call.
pub struct State<'r, T: Send + Sync + 'static>(&'r T);

impl<'r, T: Send + Sync + 'static> State<'r, T> {
    pub fn new(t: &'r T) -> Self {
        State(t)
    }
    pub fn inner(&self) -> &'r T {
        self.0
    }
}

impl<T: Send + Sync + 'static> Deref for State<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0
    }
}

impl<T: Send + Sync + 'static> Clone for State<'_, T> {
    fn clone(&self) -> Self {
        State(self.0)
    }
}

/// Where Tauri puts the app's folders.
#[derive(Clone, Debug)]
pub struct AppHandle {
    pub identifier: String,
}

impl AppHandle {
    pub fn new(identifier: impl Into<String>) -> Self {
        Self { identifier: identifier.into() }
    }
}

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub struct PathResolver<'a>(&'a AppHandle);

impl PathResolver<'_> {
    /// `~/Library/Caches/<identifier>` on macOS, `$XDG_CACHE_HOME/<identifier>` elsewhere.
    pub fn app_cache_dir(&self) -> Result<PathBuf, Error> {
        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| Error("no home folder".into()))?;
        #[cfg(target_os = "macos")]
        let base = home.join("Library/Caches");
        #[cfg(not(target_os = "macos"))]
        let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".cache"));
        Ok(base.join(&self.0.identifier))
    }
}

pub trait Manager {
    fn path(&self) -> PathResolver<'_>;
}

impl Manager for AppHandle {
    fn path(&self) -> PathResolver<'_> {
        PathResolver(self)
    }
}

pub mod ipc {
    use std::sync::Arc;

    /// A command's stream back to the UI: here, a callback.
    pub struct Channel<T>(Arc<dyn Fn(T) + Send + Sync>);

    impl<T> Channel<T> {
        pub fn new(f: impl Fn(T) + Send + Sync + 'static) -> Self {
            Channel(Arc::new(f))
        }
        pub fn send(&self, t: T) -> Result<(), crate::Error> {
            (self.0)(t);
            Ok(())
        }
    }

    impl<T> Clone for Channel<T> {
        fn clone(&self) -> Self {
            Channel(self.0.clone())
        }
    }

    /// Raw bytes back from a command.
    pub struct Response(Vec<u8>);

    impl Response {
        pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
            Response(bytes.into())
        }
        pub fn into_bytes(self) -> Vec<u8> {
            self.0
        }
    }
}

pub mod async_runtime {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    #[derive(Debug)]
    pub struct JoinError;

    impl std::fmt::Display for JoinError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("the blocking task panicked")
        }
    }

    impl std::error::Error for JoinError {}

    pub struct JoinHandle<T>(tokio::sync::oneshot::Receiver<T>);

    impl<T> Future for JoinHandle<T> {
        type Output = Result<T, JoinError>;
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            Pin::new(&mut self.0).poll(cx).map(|r| r.map_err(|_| JoinError))
        }
    }

    /// Run `f` on its own OS thread.
    pub fn spawn_blocking<F, T>(f: F) -> JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        JoinHandle(rx)
    }
}
