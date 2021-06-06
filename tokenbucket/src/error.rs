use thiserror::Error;

/// Error type
#[derive(Debug, Error)]
pub enum Error {
    #[error("TokenBucket is already running")]
    AlreadyRunning,
    #[error("error during channel send")]
    ChannelError(#[from] crossbeam_channel::SendError<()>),
    #[error("error during channel recv")]
    RecvError(#[from] crossbeam_channel::RecvError),
    #[error("checked division on time failed")]
    DivError,
    #[error("semaphore error")]
    SemaphoreError(#[from] tokio::sync::AcquireError),
}
