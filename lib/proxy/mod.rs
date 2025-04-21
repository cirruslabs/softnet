mod exposed_port;
mod host;
mod port_forwarder;
mod udp_packet_helper;
mod vm;

use crate::dhcp_snooper::DhcpSnooper;
use crate::host::Host;
use crate::host::NetType;
use crate::poller::Poller;
use crate::vm::VM;
use anyhow::Result;
pub use exposed_port::ExposedPort;
use ipnet::Ipv4Net;
use mac_address::MacAddress;
use port_forwarder::PortForwarder;
use prefix_trie::{Prefix, PrefixSet};
use smoltcp::wire::EthernetFrame;
use std::io::ErrorKind;
use std::os::unix::io::{AsRawFd, RawFd};

pub struct Proxy<'proxy> {
    vm: VM,
    host: Host,
    poller: Poller<'proxy>,
    vm_mac_address: smoltcp::wire::EthernetAddress,
    dhcp_snooper: DhcpSnooper,
    allow: PrefixSet<Ipv4Net>,
    enobufs_encountered: bool,
    port_forwarder: PortForwarder,
}

impl Proxy<'_> {
    pub fn new<'proxy>(
        vm_fd: RawFd,
        vm_mac_address: MacAddress,
        vm_net_type: NetType,
        allow: PrefixSet<Ipv4Net>,
        exposed_ports: Vec<ExposedPort>,
    ) -> Result<Proxy<'proxy>> {
        let vm = VM::new(vm_fd)?;
        let host = Host::new(vm_net_type, !allow.contains(&Ipv4Net::zero()))?;
        let poller = Poller::new(vm.as_raw_fd(), host.as_raw_fd())?;

        Ok(Proxy {
            vm,
            host,
            poller,
            vm_mac_address: smoltcp::wire::EthernetAddress(vm_mac_address.bytes()),
            dhcp_snooper: Default::default(),
            allow,
            enobufs_encountered: false,
            port_forwarder: PortForwarder::new(exposed_ports),
        })
    }

    pub fn run(&mut self) -> Result<()> {
        let mut buf: Vec<u8> = vec![0; self.host.max_packet_size as usize];

        self.poller.arm()?;

        loop {
            let (vm_readable, host_readable, interrupt) = self.poller.wait()?;

            if vm_readable {
                self.read_from_vm(buf.as_mut_slice())?;
            }

            if host_readable {
                self.read_from_host(buf.as_mut_slice())?;
            }

            // Graceful termination
            if interrupt {
                return Ok(());
            }

            // Timeout
            if !vm_readable && !host_readable && !interrupt {
                self.port_forwarder
                    .tick(&mut self.host, self.dhcp_snooper.lease());
            }

            self.poller.rearm();
        }
    }

    fn read_from_vm(&mut self, buf: &mut [u8]) -> Result<()> {
        loop {
            match self.vm.read(buf) {
                Ok(n) => {
                    if let Ok(frame) = EthernetFrame::new_checked(&buf[..n]) {
                        self.process_frame_from_vm(frame)?;
                    }
                }
                Err(err) => {
                    if err.kind() == ErrorKind::WouldBlock {
                        return Ok(());
                    }

                    return Err(err.into());
                }
            }
        }
    }

    fn read_from_host(&mut self, buf: &mut [u8]) -> Result<()> {
        loop {
            match self.host.read(buf) {
                Ok(n) => {
                    if let Ok(pkt) = EthernetFrame::new_checked(&buf[..n]) {
                        self.process_frame_from_host(&pkt)?;
                    }
                }
                Err(err) => {
                    if let vmnet::Error::VmnetReadNothing = err {
                        return Ok(());
                    }

                    return Err(err.into());
                }
            }
        }
    }
}
