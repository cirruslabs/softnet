use crate::dhcp_snooper::Lease;
use crate::host::Host;
use crate::proxy::exposed_port::ExposedPort;
use anyhow::Result;
use log::error;
use std::net::Ipv4Addr;

#[derive(Default)]
pub struct PortForwarder {
    port_forwardings: Vec<PortForwarding>,
    failed: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct PortForwarding {
    exposed_port: ExposedPort,
    forwarding_to_addr: Option<Ipv4Addr>,
}

impl PortForwarder {
    pub fn new(exposed_ports: Vec<ExposedPort>) -> PortForwarder {
        let port_forwardings = exposed_ports
            .into_iter()
            .map(|exposed_port| PortForwarding {
                exposed_port,
                ..Default::default()
            })
            .collect();

        PortForwarder {
            port_forwardings,
            ..Default::default()
        }
    }

    pub fn tick(&mut self, host: &mut Host, lease: &Option<Lease>) {
        if self.failed {
            return;
        }

        if let Err(err) = self.tick_inner(host, lease) {
            error!("port-forwarding failed: {}", err);

            self.failed = true;
        }
    }

    fn tick_inner(&mut self, host: &mut Host, lease: &Option<Lease>) -> Result<()> {
        if let Some(lease) = lease {
            // Lease exists, but is not valid, remove all port forwardings
            if !lease.valid() {
                self.remove_all_port_forwardings(host)?;

                return Ok(());
            }

            // Lease exists and is valid, install/re-install port forwardings
            for port_forwarding in &mut self.port_forwardings {
                if let Some(installed_addr) = port_forwarding.forwarding_to_addr {
                    // Port forwarding already installed, perhaps it's outdated?
                    if installed_addr == lease.address() {
                        // Nope, the port forwarding is up to date
                        continue;
                    }

                    // Remove port forwarding since the lease address had changed
                    host.port_forwarding_remove_rule(port_forwarding.exposed_port.external_port)?;
                    port_forwarding.forwarding_to_addr = None;
                }

                // Install new port forwarding
                host.port_forwarding_add_rule(
                    port_forwarding.exposed_port.external_port,
                    lease.address(),
                    port_forwarding.exposed_port.internal_port,
                )?;
                port_forwarding.forwarding_to_addr = Some(lease.address());
            }
        } else {
            // Lease does not exist, remove all port forwardings
            self.remove_all_port_forwardings(host)?;
        }

        Ok(())
    }

    fn remove_all_port_forwardings(&mut self, host: &mut Host) -> Result<()> {
        for port_forwarding in &mut self.port_forwardings {
            if port_forwarding.forwarding_to_addr.is_none() {
                continue;
            }

            host.port_forwarding_remove_rule(port_forwarding.exposed_port.external_port)?;
            port_forwarding.forwarding_to_addr = None;
        }

        Ok(())
    }
}
