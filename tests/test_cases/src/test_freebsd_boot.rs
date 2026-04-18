use macros::{guest, host};

pub struct TestFreeBsdBoot;

#[host]
mod host {
    use super::*;

    use crate::common_freebsd::{freebsd_assets, setup_kernel_and_enter};
    use crate::{krun_call_u32, ShouldRun, Test, TestSetup};
    use krun_sys::*;

    impl Test for TestFreeBsdBoot {
        fn start_vm(self: Box<Self>, test_setup: TestSetup) -> anyhow::Result<()> {
            let assets =
                freebsd_assets().expect("FreeBSD assets must be present when test runs");
            unsafe {
                let ctx = krun_call_u32!(krun_create_ctx())?;
                setup_kernel_and_enter(ctx, test_setup, assets)?;
            }
            Ok(())
        }

        fn should_run(&self) -> ShouldRun {
            ShouldRun::requires_freebsd_assets(
                "KRUN_TEST_FREEBSD_KERNEL_PATH / KRUN_TEST_FREEBSD_ISO_PATH not set or files missing",
            )
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
