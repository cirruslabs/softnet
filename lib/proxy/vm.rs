use crate::proxy::udp_packet_helper::UdpPacketHelper;
use crate::proxy::Proxy;
use crate::{Error, Result};
use smoltcp::wire::{
    ArpPacket, EthernetFrame, EthernetProtocol, IpProtocol, Ipv4Packet, UdpPacket,
};
use std::net::Ipv4Addr;

impl Proxy {
    pub(crate) fn process_frame_from_vm(&mut self, frame: EthernetFrame<&[u8]>) -> Result<()> {
        if self.allowed_from_vm(&frame).is_none() {
            // Block packet by not forwarding it to the host
            return Ok(());
        }

        self.host
            .write(frame.as_ref())
            .map(|_| ())
            .map_err(|err| Error::HostIOFailed { source: err })
    }

    fn allowed_from_vm(&self, frame: &EthernetFrame<&[u8]>) -> Option<()> {
        if frame.src_addr() != self.vm_mac_address {
            return None;
        }

        match frame.ethertype() {
            EthernetProtocol::Arp => {
                let arp_pkt = ArpPacket::new_checked(frame.payload()).ok()?;
                self.allowed_from_vm_arp(arp_pkt)
            }
            EthernetProtocol::Ipv4 => {
                let ipv4_pkt = Ipv4Packet::new_checked(frame.payload()).ok()?;
                self.allowed_from_vm_ipv4(ipv4_pkt)
            }
            _ => None,
        }
    }

    fn allowed_from_vm_arp(&self, arp_pkt: ArpPacket<&[u8]>) -> Option<()> {
        if arp_pkt.source_hardware_addr() != self.vm_mac_address.0 {
            return None;
        }

        let source_protocol_addr: [u8; 4] = arp_pkt.source_protocol_addr().try_into().unwrap();
        let source_protocol_addr = Ipv4Addr::from(source_protocol_addr);

        if let Some(lease) = self.dhcp_snooper.lease() {
            if lease.valid_ip_source(source_protocol_addr.into()) {
                return Some(());
            }
        } else if source_protocol_addr.is_unspecified() {
            return Some(());
        }

        None
    }

    fn allowed_from_vm_ipv4(&self, ipv4_pkt: Ipv4Packet<&[u8]>) -> Option<()> {
        // Once we've learned the VM's IP from the DHCP snooping,
        // allow all global traffic for that VM's IP
        if let Some(lease) = &self.dhcp_snooper.lease() {
            let dst_is_global =
                ip_network::IpNetwork::from(Ipv4Addr::from(ipv4_pkt.dst_addr().0)).is_global();

            if lease.valid_ip_source(ipv4_pkt.src_addr()) && dst_is_global {
                return Some(());
            }
        }

        // Allow communication with host
        if ipv4_pkt.dst_addr() == self.host.gateway_ip {
            return Some(());
        }

        if ipv4_pkt.protocol() == IpProtocol::Udp {
            let udp_pkt = UdpPacket::new_checked(ipv4_pkt.payload()).ok()?;

            // Allow DNS communication with the DNS-servers provided by DHCP
            if udp_pkt.is_dns_request() && self.dhcp_snooper.valid_dns_target(&ipv4_pkt.dst_addr())
            {
                return Some(());
            }

            // Allow DHCP communication with the bootpd(8) on host via broadcast address
            if udp_pkt.is_dhcp_request() && ipv4_pkt.dst_addr().is_broadcast() {
                return Some(());
            }
        }

        None
    }
}
