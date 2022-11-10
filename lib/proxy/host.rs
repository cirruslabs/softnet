use crate::proxy::udp_packet_helper::UdpPacketHelper;
use crate::proxy::Proxy;
use crate::{Error, Result};
use smoltcp::wire::{EthernetFrame, EthernetProtocol, Ipv4Packet, UdpPacket};

impl Proxy {
    pub(crate) fn process_frame_from_host(&mut self, frame: &EthernetFrame<&[u8]>) -> Result<()> {
        if self.allowed_from_host(frame).is_none() {
            // Block packet by not forwarding it to the VM
            return Ok(());
        }

        // Snoop bootpd(8) replies from the host to
        // figure out the IP assigned to the VM
        if frame.dst_addr() == self.vm_mac_address {
            self.snoop(frame);
        }

        self.vm
            .write(frame.as_ref())
            .map(|_| ())
            .map_err(|err| Error::VMIOFailed { source: err })
    }

    fn allowed_from_host(&mut self, frame: &EthernetFrame<&[u8]>) -> Option<()> {
        match frame.ethertype() {
            EthernetProtocol::Arp => Some(()),
            EthernetProtocol::Ipv4 => Some(()),
            _ => None,
        }
    }

    fn snoop(&mut self, frame: &EthernetFrame<&[u8]>) {
        if frame.ethertype() != EthernetProtocol::Ipv4 {
            return;
        }

        let ipv4_pkt = match Ipv4Packet::new_checked(frame.payload()) {
            Ok(ipv4_pkt) => ipv4_pkt,
            _ => return,
        };

        if ipv4_pkt.src_addr() != self.host.gateway_ip {
            return;
        }

        if ipv4_pkt.protocol() != smoltcp::wire::IpProtocol::Udp {
            return;
        }

        let udp_pkt = match UdpPacket::new_checked(ipv4_pkt.payload()) {
            Ok(udp_pkt) => udp_pkt,
            Err(_) => return,
        };

        if !udp_pkt.is_dhcp_response() {
            return;
        }

        self.dhcp_snooper.register_dhcp_reply(udp_pkt.payload());
    }
}
