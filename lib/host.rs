use anyhow::{anyhow, Context, Result};
use clap::ValueEnum;
use std::net::Ipv4Addr;
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixDatagram;
use std::str::FromStr;
use std::sync::mpsc::{sync_channel, SyncSender};
use vmnet::mode::Mode;
use vmnet::parameters::{Parameter, ParameterKind};
use vmnet::{Events, Options};

#[derive(ValueEnum, Clone, Debug)]
pub enum NetType {
    /// Shared network
    ///
    /// Uses NAT-translation to give guests access to the global network
    Nat,
    /// Host network
    ///
    /// Guests will be able to talk only to the host without access to global network
    Host,
}

pub struct Host {
    interface: vmnet::Interface,
    new_packets_rx: UnixDatagram,
    callback_can_continue_tx: SyncSender<()>,
    pub gateway_ip: smoltcp::wire::Ipv4Address,
    pub max_packet_size: u64,
    finalized: bool,
}

impl Host {
    pub fn new(vm_net_type: NetType, enable_isolation: bool) -> Result<Host> {
        // Initialize a vmnet.framework NAT or Host interface with isolation enabled
        let mut interface = vmnet::Interface::new(
            match vm_net_type {
                NetType::Nat => Mode::Shared(Default::default()),
                NetType::Host => Mode::Host(Default::default()),
            },
            Options {
                enable_isolation: Some(enable_isolation),
                ..Default::default()
            },
        )
        .context("failed to initialize vmnet interface")?;

        // Retrieve first IP (gateway) used for this interface
        let Some(Parameter::StartAddress(gateway_ip)) =
            interface.parameters().get(ParameterKind::StartAddress)
        else {
            return Err(anyhow!(
                "failed to retrieve vmnet's interface start address"
            ));
        };
        let gateway_ip = Ipv4Addr::from_str(&gateway_ip)
            .context("failed to parse vmnet's interface start address")?;

        // Retrieve max packet size for this interface
        let Some(Parameter::MaxPacketSize(max_packet_size)) =
            interface.parameters().get(ParameterKind::MaxPacketSize)
        else {
            return Err(anyhow!(
                "failed to retrieve vmnet's interface max packet size"
            ));
        };

        // Set up a socketpair() to emulate polling of the vmnet interface
        let (new_packets_tx, new_packets_rx) = UnixDatagram::pair()?;
        new_packets_rx.set_nonblocking(true)?;

        let (callback_can_continue_tx, callback_can_continue_rx) = sync_channel(0);

        interface
            .set_event_callback(Events::PACKETS_AVAILABLE, move |_mask, _params| {
                // Send a dummy datagram to make the other end of socketpair() readable
                // and ignore the error as this merely a signalling channel to wake up
                // the poller
                new_packets_tx.send(&[0; 1]).ok();

                // Wait for the permission to continue to avoid
                // wasting CPU cycles or in case of termination,
                // to unblock this Block[1] and allow
                // vmnet.framework to terminate
                //
                // [1]: https://en.wikipedia.org/wiki/Blocks_(C_language_extension)
                callback_can_continue_rx.recv().unwrap();
            })
            .context("failed to set vmnet interface's event callback")?;

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
            .context("failed to clear vmnet interface's event callback")?;

        // Now let the callback finish
        let _ = self.callback_can_continue_tx.send(());

        self.interface
            .finalize()
            .context("failed to finalize vmnet's interface")?;

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
