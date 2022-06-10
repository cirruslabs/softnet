mod dhcp_snooper;
mod host;
mod poller;
pub mod proxy;
mod vm;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("initialization failed")]
    InitFailed { source: Box<dyn std::error::Error> },

    #[error("failed to poll")]
    PollFailed { source: std::io::Error },

    #[error("vmnet failed")]
    VmnetFailed { source: vmnet::Error },

    #[error("vmnet returned unexpected data")]
    VmnetUnexpected,

    #[error("failed to do I/O on VM socket")]
    VMIOFailed { source: std::io::Error },

    #[error("failed to do I/O on host socket")]
    HostIOFailed { source: vmnet::Error },
}

pub type Result<T> = std::result::Result<T, Error>;
