use crate::{Error, Result};
use std::net::Ipv4Addr;
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixDatagram;
use std::str::FromStr;
use std::sync::mpsc::{sync_channel, SyncSender};
use vmnet::mode::Mode;
use vmnet::parameters::{Parameter, ParameterKind};
use vmnet::{Events, Options};

pub struct Host {
    interface: vmnet::Interface,
    new_packets_rx: UnixDatagram,
    callback_can_continue_tx: SyncSender<()>,
    pub gateway_ip: smoltcp::wire::Ipv4Address,
    pub max_packet_size: u64,
    finalized: bool,
}

impl Host {
    pub fn new() -> Result<Host> {
        // Initialize a vmnet.framework NAT interface with isolation enabled
        let mut interface = vmnet::Interface::new(
            Mode::Shared(Default::default()),
            Options {
                enable_isolation: Some(true),
                ..Default::default()
            },
        )
        .map_err(|err| Error::VmnetFailed { source: err })?;

        // Retrieve first IP (gateway) used for this interface
        let gateway_ip = match interface.parameters().get(ParameterKind::StartAddress) {
            Some(Parameter::StartAddress(gateway_ip)) => gateway_ip,
            _ => return Err(Error::VmnetUnexpected),
        };
        let gateway_ip = Ipv4Addr::from_str(&gateway_ip).map_err(|_| Error::VmnetUnexpected)?;

        // Retrieve max packet size for this interface
        let max_packet_size = match interface.parameters().get(ParameterKind::MaxPacketSize) {
            Some(Parameter::MaxPacketSize(max_packet_size)) => max_packet_size,
            _ => return Err(Error::VmnetUnexpected),
        };

        // Set up a socketpair() to emulate polling of the vmnet interface
        let (new_packets_tx, new_packets_rx) =
            UnixDatagram::pair().map_err(|err| Error::InitFailed { source: err.into() })?;
        new_packets_rx
            .set_nonblocking(true)
            .map_err(|err| Error::InitFailed { source: err.into() })?;

        let (callback_can_continue_tx, callback_can_continue_rx) = sync_channel(0);

        interface
            .set_event_callback(Events::PACKETS_AVAILABLE, move |_mask, _params| {
                // Send a dummy datagram to make the other end of socketpair() readable
                new_packets_tx.send(&[0; 1]).unwrap();

                // Wait for the permission to continue to avoid
                // wasting CPU cycles or in case of termination,
                // to unblock this Block[1] and allow
                // vmnet.framework to terminate
                //
                // [1]: https://en.wikipedia.org/wiki/Blocks_(C_language_extension)
                callback_can_continue_rx.recv().unwrap();
            })
            .map_err(|err| Error::VmnetFailed { source: err })?;

        Ok(Host {
            interface,
            new_packets_rx,
            callback_can_continue_tx,
            gateway_ip: gateway_ip.into(),
            max_packet_size,
            finalized: false,
        })
    }
}

impl Host {
    pub fn read(&mut self, buf: &mut [u8]) -> vmnet::Result<usize> {
        // Dequeue dummy datagram from the socket (if any)
        // to free up buffer space and reduce false-positives
        // when polling
        let mut buf_to_be_discarded: [u8; 1] = [0; 1];
        let _ = self.new_packets_rx.recv(&mut buf_to_be_discarded);

        let result = self.interface.read(buf);

        if let Err(vmnet::Error::VmnetReadNothing) = result {
            // We've emptied everything, unlock the callback
            // so that it will be able to pick up new events
            let _ = self.callback_can_continue_tx.send(());
        }

        result
    }

    pub fn write(&mut self, buf: &[u8]) -> vmnet::Result<usize> {
        self.interface.write(buf)
    }

    pub fn finalize(&mut self) -> Result<()> {
        // First make sure our callback won't be scheduled again after it finishes
        self.interface
            .clear_event_callback()
            .map_err(|err| Error::VmnetFailed { source: err })?;

        // Now let the callback finish
        let _ = self.callback_can_continue_tx.send(());

        self.interface
            .finalize()
            .map_err(|err| Error::VmnetFailed { source: err })?;

        self.finalized = true;

        Ok(())
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if !self.finalized {
            let _ = self.finalize();
        }
    }
}

impl AsRawFd for Host {
    fn as_raw_fd(&self) -> RawFd {
        self.new_packets_rx.as_raw_fd()
    }
}
