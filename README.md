# Softnet

Softnet is a software networking for [Tart](https://github.com/cirruslabs/tart) which provides better network isolation and alleviates DHCP shortage on production systems.

It is essentially a userspace packet filter which restricts the VM networking and prevents a class of security issues, such as ARP spoofing. By default, the VM will only be able to:

* send traffic from its own MAC-address
* send traffic from the IP-address assigned to it by the DHCP
* send traffic to globally routable IPv4 addresses
* send traffic to gateway IP of the vmnet bridge (this would normally be \"bridge100\" interface)
* receive any incoming traffic

In addition, Softnet tunes macOS built-in DHCP server to decrease its lease time from the default 86,400 seconds (one day) to 600 seconds (10 minutes). This is especially important when you use Tart to clone and run a lot of ephemeral VMs over a period of one day.

Please check out [this blog post](https://cirrus-ci.org/blog/2022/07/07/isolating-network-between-tarts-macos-virtual-machines/) for backstory.

## Working model

Softnet solves two problems:

1. VM network isolation
    * [`VZNATNetworkDeviceAttachment`](https://developer.apple.com/documentation/virtualization/vznatnetworkdeviceattachment) (the default networking in Tart) enables [vmnet's bridge isolation](https://developer.apple.com/documentation/vmnet/vmnet_enable_isolation_key) by default and prevents cross-VM traffic, however it's still possible for any VM to spoof the host's ARP-table and capture other VMs traffic by using tools that enable conducting the [ARP spoofing attacks](https://en.wikipedia.org/wiki/ARP_spoofing) (e.g. [arpspoof](https://www.monkey.org/~dugsong/dsniff/), [arpoison](http://www.arpoison.net/) and so on)
2. DHCP exhaustion
    * macOS built-in DHCP-server allocates a `/24` subnet with 86400 seconds lease time by default, which only allows for ~253 VMs a day (or 1 VM every ~6 minutes) to be spawned without causing a denial-of-service, which is pretty limiting for CI services like Cirrus CI

And assumes that:

1. Tart gives it's VMs unique MAC-addresses
2. macOS built-in DHCP-server won't re-use the IP-addresses from it's pool until their lease expire

...otherwise it's possible for two VMs to receive an identical IP-address from the macOS built-in DHCP-server (even in the presence of Softnet's packet filtering) and thus bypass the protections offered by Softnet.

## Installing

For proper functioning, Softnet binary requires two things:

* a [SUID-bit](https://en.wikipedia.org/wiki/Setuid#SUID) to be set on the binary or a [passwordless sudo](https://serverfault.com/questions/160581/how-to-setup-passwordless-sudo-on-linux) to be configured, which effectively gives the binary `root` privileges
  * these privileges are needed to create [`vmnet.framework`](https://developer.apple.com/documentation/vmnet) interface and perform DHCP-related system tweaks
  * the privileges will be dropped automatically to that of the calling user (or those represented by the `--user` and `--group` command-line arguments) once all of the initialization is completed
* the binary to be available in `PATH`
  * so that the Tart will be able to find it

## Running

Softnet is started and managed automatically by Tart if `--net-softnet` flag is provided when calling `tart run`.
