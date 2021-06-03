use std::sync::mpsc::SendError;

use thiserror::Error;

/// Error type
#[derive(Debug, Error)]
pub enum Error {
    #[error("TokenBucket is already running")]
    AlreadyRunning,
    #[error("error during channel send")]
    ChannelError(#[from] SendError<()>),
}
