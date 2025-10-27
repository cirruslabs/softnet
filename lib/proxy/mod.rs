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
use prefix_trie::{Prefix, PrefixMap, PrefixSet};
use smoltcp::wire::EthernetFrame;
use std::io::ErrorKind;
use std::os::unix::io::{AsRawFd, RawFd};
use vmnet::Batch;

pub struct Proxy<'proxy> {
    vm: VM,
    host: Host,
    poller: Poller<'proxy>,
    vm_mac_address: smoltcp::wire::EthernetAddress,
    dhcp_snooper: DhcpSnooper,
    rules: PrefixMap<Ipv4Net, Action>,
    enobufs_encountered: bool,
    port_forwarder: PortForwarder,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Action {
    Block,
    Allow,
}

impl Proxy<'_> {
    pub fn new<'proxy>(
        vm_fd: RawFd,
        vm_mac_address: MacAddress,
        vm_net_type: NetType,
        allow: PrefixSet<Ipv4Net>,
        block: PrefixSet<Ipv4Net>,
        exposed_ports: Vec<ExposedPort>,
    ) -> Result<Proxy<'proxy>> {
        let vm = VM::new(vm_fd)?;
        let host = Host::new(vm_net_type, !allow.contains(&Ipv4Net::zero()))?;
        let poller = Poller::new(vm.as_raw_fd(), host.as_raw_fd())?;

        // Craft packet filter rules
        //
        // SECURITY: blocking rules must always take precedence
        // over allowing rules when prefixes are identical.
        let mut rules = PrefixMap::new();

        for allow_net in allow {
            rules.insert(allow_net, Action::Allow);
        }

        for block_net in block {
            rules.insert(block_net, Action::Block);
        }

        Ok(Proxy {
            vm,
            host,
            poller,
            vm_mac_address: smoltcp::wire::EthernetAddress(vm_mac_address.bytes()),
            dhcp_snooper: Default::default(),
            rules,
            enobufs_encountered: false,
            port_forwarder: PortForwarder::new(exposed_ports),
        })
    }

    pub fn run(&mut self) -> Result<()> {
        // Create a single buffer from reading from the VM
        let mut buf: Vec<u8> = vec![0; self.host.max_packet_size as usize];

        // Create multiple buffers and a batch for reading from the host
        let mut bufs = vec![
            vec![0u8; self.host.max_packet_size as usize];
            self.host.read_max_packets as usize
        ];
        let mut batch = Batch::preallocate(bufs.len());

        self.poller.arm()?;

        loop {
            let (vm_readable, host_readable, interrupt) = self.poller.wait()?;

            if vm_readable {
                self.read_from_vm(buf.as_mut_slice())?;
            }

            if host_readable {
                self.read_from_host(&mut batch, &mut bufs)?;
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

    fn read_from_host(&mut self, batch: &mut Batch, bufs: &mut [Vec<u8>]) -> Result<()> {
        loop {
            match self.host.read(batch, bufs) {
                Ok(pktcnt) => {
                    for buf in batch.packet_sized_bufs(bufs).take(pktcnt) {
                        if let Ok(pkt) = EthernetFrame::new_checked(buf) {
                            self.process_frame_from_host(&pkt)?;
                        }
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

#[cfg(test)]
mod tests {
    use crate::NetType;
    use crate::dhcp_snooper::Lease;
    use crate::proxy::{Action, Proxy};
    use ipnet::Ipv4Net;
    use mac_address::MacAddress;
    use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};
    use prefix_trie::{PrefixMap, PrefixSet};
    use serial_test::serial;
    use smoltcp::wire::{Ipv4Address, Ipv4Packet};
    use std::collections::HashSet;
    use std::os::fd::AsRawFd;
    use std::str::FromStr;
    use std::time::Duration;

    #[test]
    #[serial]
    fn test_blocking_takes_precedence() {
        let vm_ip = Ipv4Address::from_str("192.168.0.2").unwrap();
        let proxy = create_proxy(vm_ip, vec!["66.66.0.0/16"], vec!["66.66.0.0/16"]);

        assert_eq!(
            proxy.rules,
            PrefixMap::<Ipv4Net, Action>::from_iter(vec![(
                Ipv4Net::from_str("66.66.0.0/16").unwrap(),
                Action::Block
            ),])
        );

        assert!(allowed_from_vm_ipv4(&proxy, vm_ip, "66.66.66.66").is_none());
    }

    #[test]
    #[serial]
    fn test_longest_prefix_match_wins() {
        let vm_ip = Ipv4Address::from_str("192.168.0.2").unwrap();
        let proxy = create_proxy(vm_ip, vec!["33.33.33.33/32"], vec!["33.33.33.0/24"]);

        assert_eq!(
            proxy.rules,
            PrefixMap::<Ipv4Net, Action>::from_iter(vec![
                (Ipv4Net::from_str("33.33.33.33/32").unwrap(), Action::Allow),
                (Ipv4Net::from_str("33.33.33.0/24").unwrap(), Action::Block),
            ])
        );

        assert!(allowed_from_vm_ipv4(&proxy, vm_ip, "33.33.33.32").is_none());
        assert!(allowed_from_vm_ipv4(&proxy, vm_ip, "33.33.33.33").is_some());
        assert!(allowed_from_vm_ipv4(&proxy, vm_ip, "33.33.33.34").is_none());
    }

    fn create_proxy<'test>(vm_ip: Ipv4Address, allow: Vec<&str>, block: Vec<&str>) -> Proxy<'test> {
        let (vm_fd, _) = socketpair(
            AddressFamily::Unix,
            SockType::Datagram,
            None,
            SockFlag::empty(),
        )
        .unwrap();
        let vm_fd = Box::leak(Box::new(vm_fd));

        let mut proxy = Proxy::new(
            vm_fd.as_raw_fd(),
            MacAddress::from_str("02:00:00:00:00:01").unwrap(),
            NetType::Nat,
            PrefixSet::from_iter(
                allow
                    .into_iter()
                    .map(|cidr| Ipv4Net::from_str(cidr).unwrap()),
            ),
            PrefixSet::from_iter(
                block
                    .into_iter()
                    .map(|cidr| Ipv4Net::from_str(cidr).unwrap()),
            ),
            Vec::default(),
        )
        .unwrap();

        proxy.dhcp_snooper.set_lease(Some(Lease::new(
            vm_ip,
            Duration::from_secs(600),
            HashSet::new(),
        )));

        proxy
    }

    fn allowed_from_vm_ipv4(proxy: &Proxy, src: Ipv4Address, dst: &str) -> Option<()> {
        let mut buf = vec![0; 1500];

        let mut ipv4_pkt_mut = Ipv4Packet::new_unchecked(&mut buf[..]);
        ipv4_pkt_mut.set_src_addr(src);
        ipv4_pkt_mut.set_dst_addr(Ipv4Address::from_str(dst).unwrap());

        let ipv4_pkt = Ipv4Packet::new_unchecked(buf.as_slice());

        proxy.allowed_from_vm_ipv4(ipv4_pkt)
    }
}
