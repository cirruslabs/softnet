use anyhow::Result;
use num_enum::IntoPrimitive;
use polling::os::kqueue::PollerKqueueExt;
use polling::PollMode;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::os::unix::io::RawFd;
use std::time::Duration;

pub struct Poller<'poller> {
    poller: polling::Poller,
    events: polling::Events,
    vm_fd: BorrowedFd<'poller>,
    host_fd: BorrowedFd<'poller>,
}

#[derive(IntoPrimitive)]
#[repr(usize)]
enum EventKey {
    VM,
    Host,
    Interrupt,
}

impl Poller<'_> {
    pub fn new<'poller>(vm_fd: RawFd, host_fd: RawFd) -> Result<Poller<'poller>> {
        let poller = polling::Poller::new()?;

        Ok(Poller {
            poller,
            events: polling::Events::new(),
            vm_fd: unsafe { BorrowedFd::borrow_raw(vm_fd) },
            host_fd: unsafe { BorrowedFd::borrow_raw(host_fd) },
        })
    }

    pub fn arm(&self) -> Result<()> {
        unsafe {
            self.poller
                .add(self.vm_fd.as_raw_fd(), self.vm_interest())?;
            self.poller
                .add(self.host_fd.as_raw_fd(), self.host_interest())?;
        }

        let interrupt_signal = polling::os::kqueue::Signal(libc::SIGINT);
        self.poller
            .add_filter(
                interrupt_signal,
                EventKey::Interrupt.into(),
                PollMode::Oneshot,
            )
            .unwrap();

        Ok(())
    }

    pub fn rearm(&mut self) -> Result<()> {
        self.events.clear();

        self.poller.modify(self.vm_fd, self.vm_interest())?;
        self.poller.modify(self.host_fd, self.host_interest())?;

        let interrupt_signal = polling::os::kqueue::Signal(libc::SIGINT);
        self.poller.modify_filter(
            interrupt_signal,
            EventKey::Interrupt.into(),
            PollMode::Oneshot,
        )?;

        Ok(())
    }

    pub fn wait(&mut self) -> Result<(bool, bool, bool)> {
        self.poller
            .wait(&mut self.events, Some(Duration::from_millis(100)))?;

        let vm_readable = self
            .events
            .iter()
            .any(|ev| ev.key == Into::<usize>::into(EventKey::VM));
        let host_readable = self
            .events
            .iter()
            .any(|ev| ev.key == Into::<usize>::into(EventKey::Host));
        let interrupt = self
            .events
            .iter()
            .any(|ev| ev.key == Into::<usize>::into(EventKey::Interrupt));

        Ok((vm_readable, host_readable, interrupt))
    }

    fn vm_interest(&self) -> polling::Event {
        polling::Event::readable(EventKey::VM.into())
    }

    fn host_interest(&self) -> polling::Event {
        polling::Event::readable(EventKey::Host.into())
    }
}
