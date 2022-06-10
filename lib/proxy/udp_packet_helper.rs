use smoltcp::wire::UdpPacket;

pub(crate) trait UdpPacketHelper {
    const DNS_PORT: u16 = 53;
    const BOOTPS_PORT: u16 = 67;
    const BOOTPC_PORT: u16 = 68;

    fn is_dns_request(&self) -> bool;

    fn is_dhcp_request(&self) -> bool;
    fn is_dhcp_response(&self) -> bool;
}

impl UdpPacketHelper for UdpPacket<&[u8]> {
    fn is_dns_request(&self) -> bool {
        self.dst_port() == Self::DNS_PORT
    }

    fn is_dhcp_request(&self) -> bool {
        self.src_port() == Self::BOOTPC_PORT || self.dst_port() == Self::BOOTPS_PORT
    }

    fn is_dhcp_response(&self) -> bool {
        self.src_port() == Self::BOOTPS_PORT || self.dst_port() == Self::BOOTPC_PORT
    }
}
