mod test_vm_config;
use test_vm_config::TestVmConfig;

mod test_vsock_guest_connect;
use test_vsock_guest_connect::TestVsockGuestConnect;

mod test_tsi_tcp_guest_connect;
use test_tsi_tcp_guest_connect::TestTsiTcpGuestConnect;

mod test_tsi_tcp_guest_listen;
use test_tsi_tcp_guest_listen::TestTsiTcpGuestListen;

mod test_multiport_console;
use test_multiport_console::TestMultiportConsole;

mod test_freebsd_boot;
use test_freebsd_boot::TestFreeBsdBoot;

pub enum ShouldRun {
    Yes,
    No(&'static str),
}

impl ShouldRun {
    /// Returns Yes unless on macOS, in which case returns No with the given reason.
    pub fn yes_unless_macos(reason: &'static str) -> Self {
        if cfg!(target_os = "macos") {
            ShouldRun::No(reason)
        } else {
            ShouldRun::Yes
        }
    }

    /// Returns Yes if both `KRUN_TEST_FREEBSD_KERNEL_PATH` and `KRUN_TEST_FREEBSD_ISO_PATH`
    /// are set and point to existing files, otherwise returns No with the given reason.
    #[cfg(feature = "host")]
    pub fn requires_freebsd_assets(reason: &'static str) -> Self {
        let kernel_ok = std::env::var_os("KRUN_TEST_FREEBSD_KERNEL_PATH")
            .map(|p| std::path::Path::new(&p).exists())
            .unwrap_or(false);
        let iso_ok = std::env::var_os("KRUN_TEST_FREEBSD_ISO_PATH")
            .map(|p| std::path::Path::new(&p).exists())
            .unwrap_or(false);
        if kernel_ok && iso_ok {
            ShouldRun::Yes
        } else {
            ShouldRun::No(reason)
        }
    }
}

pub fn test_cases() -> Vec<TestCase> {
    // Register your test here:
    vec![
        TestCase::new(
            "configure-vm-1cpu-256MiB",
            Box::new(TestVmConfig {
                num_cpus: 1,
                ram_mib: 256,
            }),
        ),
        TestCase::new(
            "configure-vm-2cpu-1GiB",
            Box::new(TestVmConfig {
                num_cpus: 2,
                ram_mib: 1024,
            }),
        ),
        TestCase::new("vsock-guest-connect", Box::new(TestVsockGuestConnect)),
        TestCase::new(
            "tsi-tcp-guest-connect",
            Box::new(TestTsiTcpGuestConnect::new()),
        ),
        TestCase::new(
            "tsi-tcp-guest-listen",
            Box::new(TestTsiTcpGuestListen::new()),
        ),
        TestCase::new("multiport-console", Box::new(TestMultiportConsole)),
        TestCase::new("freebsd-boot", Box::new(TestFreeBsdBoot)),
    ]
}

////////////////////
// Implementation details:
//////////////////
use macros::{guest, host};
#[host]
use std::path::PathBuf;
#[host]
use std::process::Child;

#[cfg(all(feature = "guest", feature = "host"))]
compile_error!("Cannot enable both guest and host in the same binary!");

#[cfg(feature = "host")]
mod common;

#[cfg(feature = "host")]
mod common_freebsd;

#[cfg(feature = "host")]
mod krun;
mod tcp_tester;

#[host]
#[derive(Clone, Debug)]
pub struct TestSetup {
    pub test_case: String,
    // A tmp directory for misc. artifacts used be the test (e.g. sockets)
    pub tmp_dir: PathBuf,
}

#[host]
pub trait Test {
    /// Start the VM
    fn start_vm(self: Box<Self>, test_setup: TestSetup) -> anyhow::Result<()>;

    /// Checks the output of the (host) process which started the VM
    fn check(self: Box<Self>, child: Child) {
        let output = child.wait_with_output().unwrap();
        assert_eq!(String::from_utf8(output.stdout).unwrap(), "OK\n");
    }

    /// Check if this test should run on this platform.
    fn should_run(&self) -> ShouldRun {
        ShouldRun::Yes
    }
}

#[guest]
pub trait Test {
    /// This will be executed in the guest, you can panic! if the test failed!
    fn in_guest(self: Box<Self>) {}
}

pub struct TestCase {
    pub name: &'static str,
    pub test: Box<dyn Test>,
}

impl TestCase {
    // Your test can be parametrized, so you can add the same test multiple times constructed with
    // different parameters with and specify a different name here.
    pub fn new(name: &'static str, test: Box<dyn Test>) -> Self {
        Self { name, test }
    }

    /// Check if this test should run on this platform.
    #[host]
    pub fn should_run(&self) -> ShouldRun {
        self.test.should_run()
    }

    #[allow(dead_code)]
    pub fn name(&self) -> &'static str {
        self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn all_testcases_have_unique_names() {
        let test_cases = test_cases();
        let mut names: HashSet<&str> = HashSet::new();

        for test_case in test_cases {
            let name = test_case.name();
            let was_inserted = names.insert(name);
            if !was_inserted {
                panic!("test_cases() contains multiple items named `{name}`")
            }

            if name == "all" {
                panic!("test_cases() contains test named {name}, but the name is reseved")
            }
        }
    }
}
