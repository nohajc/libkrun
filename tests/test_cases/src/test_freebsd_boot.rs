use macros::{guest, host};

pub struct TestFreeBsdBoot;

#[host]
mod host {
    use super::*;

    use std::process::Child;

    use crate::common_freebsd::{freebsd_assets, normalize_serial_output, setup_kernel_and_enter};
    use crate::{krun_call_u32, ShouldRun, Test, TestSetup};
    use krun_sys::*;

    impl Test for TestFreeBsdBoot {
        fn check(self: Box<Self>, child: Child) {
            let output = child.wait_with_output().unwrap();
            let stdout = normalize_serial_output(output.stdout);
            assert_eq!(stdout, "OK\n");
        }

        fn start_vm(self: Box<Self>, test_setup: TestSetup) -> anyhow::Result<()> {
            let assets = freebsd_assets().expect("FreeBSD assets must be present when test runs");
            unsafe {
                let ctx = krun_call_u32!(krun_create_ctx())?;
                setup_kernel_and_enter(ctx, test_setup, assets)?;
            }
            Ok(())
        }

        fn should_run(&self) -> ShouldRun {
            ShouldRun::requires_freebsd_assets("init-freebsd not compiled")
        }
    }
}

#[guest]
mod guest {
    use super::*;

    use crate::Test;

    impl Test for TestFreeBsdBoot {
        fn in_guest(self: Box<Self>) {
            println!("OK");
        }
    }
}
