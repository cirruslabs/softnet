use crate::proxy::udp_packet_helper::UdpPacketHelper;
use crate::proxy::Proxy;
use anyhow::Context;
use anyhow::Result;
use ipnet::Ipv4Net;
use smoltcp::wire::{
    ArpPacket, EthernetFrame, EthernetProtocol, IpProtocol, Ipv4Packet, UdpPacket,
};
use std::net::Ipv4Addr;

impl Proxy<'_> {
    pub(crate) fn process_frame_from_vm(&mut self, frame: EthernetFrame<&[u8]>) -> Result<()> {
        if self.allowed_from_vm(&frame).is_none() {
            // Block packet by not forwarding it to the host
            return Ok(());
        }

        self.host
            .write(frame.as_ref())
            .map(|_| ())
            .context("failed to write to the host")
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
            if lease.valid_ip_source(source_protocol_addr) {
                return Some(());
            }
        } else if source_protocol_addr.is_unspecified() {
            return Some(());
        }

        None
    }

    fn allowed_from_vm_ipv4(&self, ipv4_pkt: Ipv4Packet<&[u8]>) -> Option<()> {
        // Have we learned the VM's IP from the DHCP snooping?
        if let Some(lease) = &self.dhcp_snooper.lease() {
            let dst_addr = ipv4_pkt.dst_addr();

            // Allow traffic explicitly permitted by the user-specified CIDRs
            let dst_net = Ipv4Net::from(dst_addr);

            // Use get_lpm() instead of get_spm() to work around prefix-trie
            // not handling prefixes like 0.0.0.0/0 correctly[1]
            //
            // [1]: https://github.com/tiborschneider/prefix-trie/issues/8
            if lease.valid_ip_source(ipv4_pkt.src_addr()) && self.allow.get_lpm(&dst_net).is_some()
            {
                return Some(());
            }

            if !self.allow_only
                && lease.valid_ip_source(ipv4_pkt.src_addr())
                && ip_network::IpNetwork::from(dst_addr).is_global()
            {
                return Some(());
            }
        }

        // Allow communication with host
        if ipv4_pkt.dst_addr() == self.host.gateway_ip {
            return Some(());
        }

        if ipv4_pkt.next_header() == IpProtocol::Udp {
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
