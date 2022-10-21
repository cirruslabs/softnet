use clap::Parser;
use privdrop::PrivDrop;
use softnet::proxy::Proxy;
use std::os::raw::c_int;
use std::os::unix::io::RawFd;
use std::os::unix::process::CommandExt;
use std::process::Command;
use system_configuration::core_foundation::base::TCFType;
use system_configuration::core_foundation::dictionary::CFDictionary;
use system_configuration::core_foundation::number::CFNumber;
use system_configuration::core_foundation::string::CFString;
use system_configuration::preferences::SCPreferences;
use system_configuration::sys::preferences::{SCPreferencesCommitChanges, SCPreferencesSetValue};
use users::{get_current_groupname, get_current_username, get_effective_uid};

#[derive(Parser, Debug)]
struct Args {
    #[clap(
        long,
        help = "FD number to use for communicating with the VM's networking stack"
    )]
    vm_fd: c_int,

    #[clap(long, help = "MAC address to enforce for the VM")]
    vm_mac_address: mac_address::MacAddress,

    #[clap(
        long,
        help = "set bootpd(8) lease time to this value (in seconds) before starting the VM",
        default_value_t = 600
    )]
    bootpd_lease_time: u32,

    #[clap(long, help = "user name to drop privileges to")]
    user: Option<String>,

    #[clap(long, help = "group name to drop privileges to")]
    group: Option<String>,

    #[clap(long, hide=true)]
    sudo_escalation_probing: bool,

    #[clap(long, hide=true)]
    sudo_escalation_done: bool,
}

fn main() {
    if let Err(err) = try_main() {
        match err.source() {
            Some(source) => eprintln!("{}: {}", err, source),
            None => eprintln!("{}", err),
        }
        std::process::exit(1);
    }
}

fn try_main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Args = Args::parse();

    // No need to run anything, just return
    // so that the invoker process knows we
    // can be invoked in Sudo as root
    if args.sudo_escalation_probing {
        return Ok(());
    }

    // Retrieve real (not effective) user and group names
    let current_user_name = get_current_username()
        .ok_or("failed to resolve real user name")?
        .to_string_lossy()
        .to_string();
    let current_group_name = get_current_groupname()
        .ok_or("failed to resolve real group name")?
        .to_string_lossy()
        .to_string();

    // Ensure we are running as root
    if get_effective_uid() != 0 {
        if sudo_escalation_works() && !args.sudo_escalation_done {
            let exe = std::env::current_exe().unwrap();
            let args = std::env::args().skip(1);

            let _ = Command::new("sudo")
                .arg("-n")
                .arg(&exe)
                .args(args)
                .arg("--sudo-escalation-done")
                .arg("--user")
                .arg(current_user_name)
                .arg("--group")
                .arg(current_group_name)
                .exec();
        }

        return Err("root privileges are required to run and passwordless sudo was not available".into());
    }

    // Set bootpd(8) min/max lease time while still having the root privileges
    set_bootpd_lease_time(args.bootpd_lease_time);

    // Initialize the proxy while still having the root privileges
    let mut proxy = Proxy::new(args.vm_fd as RawFd, args.vm_mac_address)?;

    // Drop effective privileges to the user
    // and group which have had invoked us
    PrivDrop::default()
        .user(args.user.unwrap_or(current_user_name))
        .group(args.group.unwrap_or(current_group_name))
        .apply()
        .map_err(|err| format!("failed to drop privileges: {}", err))?;

    // Run proxy
    proxy.run().map_err(|err| err.into())
}

fn sudo_escalation_works() -> bool {
    let exe = std::env::current_exe().unwrap();
    let args = std::env::args().skip(1);

    Command::new("sudo")
        .arg("-n")
        .arg(&exe)
        .args(args)
        .arg("--sudo-escalation-probing")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn set_bootpd_lease_time(lease_time: u32) {
    let prefs = SCPreferences::group(
        &CFString::new("softnet"),
        &CFString::new("com.apple.InternetSharing.default.plist"),
    );

    let bootpd_dict = CFDictionary::from_CFType_pairs(&[(
        CFString::new("DHCPLeaseTimeSecs"),
        CFNumber::from(lease_time as i32),
    )]);

    unsafe {
        SCPreferencesSetValue(
            prefs.as_concrete_TypeRef(),
            CFString::new("bootpd").as_concrete_TypeRef(),
            bootpd_dict.as_concrete_TypeRef().cast(),
        );

        SCPreferencesCommitChanges(prefs.as_concrete_TypeRef());
    }
}
