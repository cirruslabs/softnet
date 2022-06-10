use crate::{Error, Result};
use num_enum::IntoPrimitive;
use std::os::unix::io::RawFd;
use std::time::Duration;

pub struct Poller {
    poller: polling::Poller,
    events: Vec<polling::Event>,
    vm_fd: RawFd,
    host_fd: RawFd,
}

#[derive(IntoPrimitive)]
#[repr(usize)]
enum EventKey {
    VM,
    Host,
}

impl Poller {
    pub fn new(vm_fd: RawFd, host_fd: RawFd) -> Result<Poller> {
        let poller =
            polling::Poller::new().map_err(|err| Error::InitFailed { source: err.into() })?;

        Ok(Poller {
            poller,
            events: Vec::new(),
            vm_fd,
            host_fd,
        })
    }

    pub fn arm(&self) -> Result<()> {
        self.poller
            .add(self.vm_fd as RawFd, self.vm_interest())
            .map_err(|err| Error::PollFailed { source: err })?;

        self.poller
            .add(self.host_fd as RawFd, self.host_interest())
            .map_err(|err| Error::PollFailed { source: err })?;

        Ok(())
    }

    pub fn rearm(&mut self) -> Result<()> {
        self.events.clear();

        self.poller
            .modify(self.vm_fd as RawFd, self.vm_interest())
            .map_err(|err| Error::PollFailed { source: err })?;
        self.poller
            .modify(self.host_fd as RawFd, self.host_interest())
            .map_err(|err| Error::PollFailed { source: err })?;

        Ok(())
    }

    pub fn wait(&mut self) -> Result<(bool, bool)> {
        self.poller
            .wait(&mut self.events, Some(Duration::from_millis(100)))
            .map_err(|err| Error::PollFailed { source: err })?;

        let vm_readable = self.events.iter().any(|ev| ev.key == EventKey::VM.into());
        let host_readable = self.events.iter().any(|ev| ev.key == EventKey::Host.into());

        Ok((vm_readable, host_readable))
    }

    fn vm_interest(&self) -> polling::Event {
        polling::Event::readable(EventKey::VM.into())
    }

    fn host_interest(&self) -> polling::Event {
        polling::Event::readable(EventKey::Host.into())
    }
}
