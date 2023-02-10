use anyhow::{anyhow, Context};
use clap::Parser;
use nix::sys::signal::{kill, SIGSTOP};
use nix::unistd::getpid;
use privdrop::PrivDrop;
use softnet::proxy::Proxy;
use std::borrow::Cow;
use std::env;
use std::os::raw::c_int;
use std::os::unix::io::RawFd;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode};
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

    #[clap(long, hide = true)]
    sudo_escalation_probing: bool,

    #[clap(long, hide = true)]
    sudo_escalation_done: bool,

    #[clap(long, hide = true)]
    sudo_escalation_interactive: bool,
}

fn main() -> ExitCode {
    // Enable backtraces by default
    if env::var("RUST_BACKTRACE").is_err() {
        env::set_var("RUST_BACKTRACE", "full");
    }

    // Initialize Sentry
    let _sentry = sentry::init(sentry::ClientOptions {
        release: option_env!("CIRRUS_TAG").map(|tag| Cow::from(format!("softnet@{tag}"))),
        ..Default::default()
    });

    // Enrich future events with Cirrus CI-specific tags
    if let Ok(tags) = env::var("CIRRUS_SENTRY_TAGS") {
        sentry::configure_scope(|scope| {
            for (key, value) in tags.split(',').filter_map(|tag| tag.split_once('=')) {
                scope.set_tag(key, value);
            }
        });
    }

    match try_main() {
        Ok(_) => ExitCode::SUCCESS,
        Err(err) => {
            // Print the error into stderr
            let causes: Vec<String> = err.chain().map(|x| x.to_string()).collect();
            eprintln!("{}", causes.join(": "));

            // Capture the error into Sentry
            sentry_anyhow::capture_anyhow(&err);

            ExitCode::FAILURE
        }
    }
}

fn try_main() -> anyhow::Result<()> {
    let args: Args = Args::parse();

    // No need to run anything, just return
    // so that the invoker process knows we
    // can be invoked in Sudo as root
    if args.sudo_escalation_probing {
        return Ok(());
    }

    // Retrieve real (not effective) user and group names
    let current_user_name = get_current_username()
        .ok_or(anyhow!("failed to resolve real user name"))?
        .to_string_lossy()
        .to_string();
    let current_group_name = get_current_groupname()
        .ok_or(anyhow!("failed to resolve real group name"))?
        .to_string_lossy()
        .to_string();

    // Ensure we are running as root
    if get_effective_uid() != 0 {
        if !args.sudo_escalation_done && args.sudo_escalation_interactive {
            eprintln!("Softnet requires a Sudo password (passwordless Sudo is not available), please enter it below.");

            Err(sudo_escalation_command(
                current_user_name.clone(),
                current_group_name.clone(),
                true,
            )
            .exec())
            .context("failed to execute Sudo")?;
        }

        if !args.sudo_escalation_done && sudo_escalation_works() {
            Err(sudo_escalation_command(current_user_name, current_group_name, false).exec())
                .context("failed to execute Sudo")?;
        }

        return Err(anyhow!(
            "root privileges are required to run, yet passwordless Sudo was not available"
        ));
    }

    // Stop ourselves and unblock the waitid(2) call in the Tart
    // to signify that we're ready to proceed after the user has
    // entered the Sudo password
    if args.sudo_escalation_interactive {
        kill(getpid(), SIGSTOP).unwrap();
    }

    // Set bootpd(8) min/max lease time while still having the root privileges
    set_bootpd_lease_time(args.bootpd_lease_time);

    // Initialize the proxy while still having the root privileges
    let mut proxy = Proxy::new(args.vm_fd as RawFd, args.vm_mac_address)
        .context("failed to initialize proxy")?;

    // Drop effective privileges to the user
    // and group which have had invoked us
    PrivDrop::default()
        .user(args.user.unwrap_or(current_user_name))
        .group(args.group.unwrap_or(current_group_name))
        .apply()
        .context("failed to drop privileges")?;

    // Run proxy
    proxy.run()
}

fn sudo_escalation_command(
    current_user_name: String,
    current_group_name: String,
    interactive: bool,
) -> Command {
    let exe = env::current_exe().unwrap();
    let args = env::args().skip(1);

    // Sudo-specific options
    let mut command = Command::new("sudo");
    if !interactive {
        command.arg("--non-interactive");
    }
    command.arg("--preserve-env=SENTRY_DSN,CIRRUS_SENTRY_TAGS");

    // Softnet-specific options
    command
        .arg(&exe)
        .args(args)
        .arg("--sudo-escalation-done")
        .arg("--user")
        .arg(current_user_name)
        .arg("--group")
        .arg(current_group_name);

    command
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
