//! Host-side utilities for FreeBSD guest tests.

use anyhow::Context;
use std::ffi::CString;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{krun_call, TestSetup};
use krun_sys::*;

pub struct FreeBsdAssets {
    pub kernel_path: PathBuf,
    pub iso_path: PathBuf,
}

/// Read FreeBSD asset paths from environment variables.
/// Returns `None` if either variable is unset or the referenced files don't exist.
pub fn freebsd_assets() -> Option<FreeBsdAssets> {
    let kernel_path = PathBuf::from(std::env::var_os("KRUN_TEST_FREEBSD_KERNEL_PATH")?);
    let iso_path = PathBuf::from(std::env::var_os("KRUN_TEST_FREEBSD_ISO_PATH")?);
    if !kernel_path.exists() || !iso_path.exists() {
        return None;
    }
    Some(FreeBsdAssets {
        kernel_path,
        iso_path,
    })
}

/// Create a `KRUN_CONFIG`-labelled ISO inside the test's tmp directory and return its path.
///
/// `init-freebsd` identifies the config disk by its ISO volume label (`/dev/iso9660/KRUN_CONFIG`),
/// not by vtbd index, so the label is mandatory.
fn create_config_iso(test_case: &str, tmp_dir: &Path) -> anyhow::Result<PathBuf> {
    let staging = tmp_dir.join("krun_config");
    std::fs::create_dir(&staging).context("create krun_config staging dir")?;

    let json = format!(r#"{{"Cmd":["/guest-agent","{test_case}"]}}"#);
    std::fs::write(staging.join("krun_config.json"), json).context("write krun_config.json")?;

    let iso_path = tmp_dir.join("krun_config.iso");
    let status = Command::new("bsdtar")
        .args([
            "cf",
            iso_path.to_str().context("config iso path is not UTF-8")?,
            "--format=iso9660",
            "--options",
            "volume-id=KRUN_CONFIG",
            "-C",
            staging.to_str().context("config staging dir is not UTF-8")?,
            "krun_config.json",
        ])
        .status()
        .context(
            "Failed to run bsdtar — on Linux install libarchive-tools; on macOS bsdtar is built-in",
        )?;

    if !status.success() {
        anyhow::bail!("bsdtar exited with {status}");
    }
    Ok(iso_path)
}

/// Boot a FreeBSD guest with `init-freebsd` and enter it.
///
/// Parallel to [`crate::common::setup_fs_and_enter`] for Linux guests:
/// - boots from a pre-built rootfs ISO (`vtbd0`) containing `init-freebsd` + `guest-agent`
/// - passes the test-case name via a `KRUN_CONFIG` ISO (`vtbd1`)
/// - uses a serial console (required by FreeBSD; output reaches the runner via the stdout pipe)
pub fn setup_kernel_and_enter(
    ctx: u32,
    test_setup: TestSetup,
    assets: FreeBsdAssets,
) -> anyhow::Result<()> {
    let config_iso = create_config_iso(&test_setup.test_case, &test_setup.tmp_dir)?;

    let kernel_cstr =
        CString::new(assets.kernel_path.as_os_str().as_bytes()).context("kernel_path CString")?;
    let rootfs_cstr =
        CString::new(assets.iso_path.as_os_str().as_bytes()).context("rootfs iso CString")?;
    let config_iso_cstr =
        CString::new(config_iso.as_os_str().as_bytes()).context("config iso CString")?;

    unsafe {
        // FreeBSD requires a serial console; virtio console is not supported.
        // The subprocess stdout (fd 1) is piped by the runner — guest serial output appears there.
        krun_call!(krun_disable_implicit_console(ctx))?;
        krun_call!(krun_add_serial_console_default(
            ctx,
            -1,
            io::stdout().as_raw_fd(),
        ))?;

        // Kernel cmdline: mount vtbd0 as root via cd9660 and hand off to init-freebsd.
        krun_call!(krun_set_kernel(
            ctx,
            kernel_cstr.as_ptr(),
            KRUN_KERNEL_FORMAT_RAW,
            std::ptr::null(),
            c"FreeBSD:vfs.root.mountfrom=cd9660:/dev/vtbd0 -mq init_path=/init-freebsd".as_ptr(),
        ))?;

        // vtbd0: rootfs ISO (init-freebsd + guest-agent)
        krun_call!(krun_add_disk(
            ctx,
            c"vtbd0".as_ptr(),
            rootfs_cstr.as_ptr(),
            true,
        ))?;

        // vtbd1: config ISO (init-freebsd finds it by KRUN_CONFIG volume label, not vtbd index)
        krun_call!(krun_add_disk(
            ctx,
            c"vtbd1".as_ptr(),
            config_iso_cstr.as_ptr(),
            true,
        ))?;

        krun_call!(krun_start_enter(ctx))?;
    }
    unreachable!()
}
