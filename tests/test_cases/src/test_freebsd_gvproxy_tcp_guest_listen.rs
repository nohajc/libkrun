use crate::tcp_tester::TcpTester;
use macros::{guest, host};
use std::net::Ipv4Addr;

const PORT: u16 = 8000;
const GUEST_IP: Ipv4Addr = Ipv4Addr::new(192, 168, 127, 2);

pub struct TestFreeBsdGvproxyTcpGuestListen {
    tcp_tester: TcpTester,
}

impl TestFreeBsdGvproxyTcpGuestListen {
    pub fn new() -> TestFreeBsdGvproxyTcpGuestListen {
        Self {
            tcp_tester: TcpTester::new_with_ip(PORT, GUEST_IP),
        }
    }
}

#[host]
mod host {
    use super::*;

    use crate::common_freebsd::{
        freebsd_assets, gvproxy_path, normalize_serial_output, setup_kernel_and_enter_with_gvproxy,
    };
    use crate::{krun_call, krun_call_u32};
    use crate::{ShouldRun, Test, TestSetup};
    use krun_sys::*;
    use std::process::Child;
    use std::thread;
    use std::time::Duration;

    impl Test for TestFreeBsdGvproxyTcpGuestListen {
        fn should_run(&self) -> ShouldRun {
            if freebsd_assets().is_none() {
                return ShouldRun::No("prerequisites not met");
            }
            if gvproxy_path().is_none() {
                return ShouldRun::No("gvproxy not available");
            }
            ShouldRun::Yes
        }

        fn start_vm(self: Box<Self>, test_setup: TestSetup) -> anyhow::Result<()> {
            let assets = freebsd_assets().expect("freebsd assets must be available");
            let gvproxy = gvproxy_path().expect("gvproxy must be available");

            // Give guest time to start listening before we try to connect
            let tcp_tester_clone = self.tcp_tester;
            thread::spawn(move || {
                thread::sleep(Duration::from_secs(3));
                tcp_tester_clone.run_client();
            });

            unsafe {
                krun_call!(krun_set_log_level(KRUN_LOG_LEVEL_INFO))?;
                let ctx = krun_call_u32!(krun_create_ctx())?;
                krun_call!(krun_set_vm_config(ctx, 1, 512))?;
                setup_kernel_and_enter_with_gvproxy(ctx, test_setup, assets, gvproxy)?;
            }
            Ok(())
        }

        fn check(self: Box<Self>, child: Child) {
            let output = child.wait_with_output().unwrap();
            let output_str = normalize_serial_output(output.stdout);
            assert_eq!(output_str, "OK\n");
        }
    }
}

#[guest]
mod guest {
    use super::*;
    use crate::freebsd_network::configure_virtio_net_ip;
    use crate::Test;

    impl Test for TestFreeBsdGvproxyTcpGuestListen {
        fn in_guest(self: Box<Self>) {
            configure_virtio_net_ip();
            self.tcp_tester
                .run_server(self.tcp_tester.create_server_socket());
            println!("OK");
        }
    }
}
