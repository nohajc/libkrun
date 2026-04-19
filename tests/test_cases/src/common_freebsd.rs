//! Host-side utilities for FreeBSD guest tests.

use anyhow::Context;
use nix::libc;
use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Find gvproxy binary path from environment or search $PATH.
pub fn gvproxy_path() -> Option<PathBuf> {
    // First check explicit env var
    if let Ok(path) = std::env::var("KRUN_TEST_GVPROXY_PATH") {
        let p = PathBuf::from(&path);
        if p.exists() {
            return Some(p);
        }
    }

    // Try to find in standard locations
    let names = if cfg!(target_os = "macos") {
        vec!["gvproxy", "gvproxy-darwin"]
    } else {
        vec!["gvproxy", "gvproxy-linux-amd64", "gvproxy-linux-arm64"]
    };

    for name in names {
        if let Ok(output) = Command::new("which").arg(name).output() {
            if output.status.success() {
                let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                return Some(PathBuf::from(path_str));
            }
        }
    }

    None
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
            staging
                .to_str()
                .context("config staging dir is not UTF-8")?,
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

/// Normalize serial-console line endings for FreeBSD output assertions.
///
/// FreeBSD's serial console emits CRLF (`\r\n`); strip the `\r` so that
/// test `check()` overrides can compare against plain `\n`-terminated strings.
pub fn normalize_serial_output(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

/// Generate a random MAC address for virtio-net device.
fn random_mac_address() -> [u8; 6] {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let mut hasher = RandomState::new().build_hasher();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    hasher.write_u32(nanos);
    let hash = hasher.finish();

    [
        0x52, // Xen OUI
        0x54,
        0x00,
        ((hash >> 16) & 0xFF) as u8,
        ((hash >> 8) & 0xFF) as u8,
        (hash & 0xFF) as u8,
    ]
}

/// Return the fixed gvproxy socket paths for a test's tmp directory.
///
/// Fixed names (rather than randomised per-run) let the parent process's `check()` know the
/// paths without any IPC.  Each test already has its own unique `tmp_dir`, so collisions between
/// concurrent tests are impossible.
///
/// The paths are kept short on purpose: macOS `sockaddr_un.sun_path` is 104 bytes including the
/// null terminator (max 103 usable chars), so unnecessarily long names inside deep tmp directories
/// overflow the limit.
pub fn gvproxy_socket_paths(tmp_dir: &Path) -> (String, String) {
    let net = tmp_dir
        .join("gvproxy-net.sock")
        .to_str()
        .expect("tmp_dir is not valid UTF-8")
        .to_string();
    let vfkit = tmp_dir
        .join("gvproxy-vfkit.sock")
        .to_str()
        .expect("tmp_dir is not valid UTF-8")
        .to_string();
    (net, vfkit)
}

/// Set up a gvproxy port-forwarding rule via its HTTP API.
///
/// Sends `POST /services/forwarder/expose` with
/// `{"local":":<port>","remote":"<remote_ip>:<port>"}` to the net unix socket.
/// Retries until gvproxy is accepting connections (up to ~10 s).
pub fn setup_gvproxy_port_forward(
    net_sock_path: &str,
    port: u16,
    remote_ip: std::net::Ipv4Addr,
) -> anyhow::Result<()> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    // Wait until gvproxy is ready to serve HTTP.
    let mut stream = None;
    for _ in 0..100 {
        match UnixStream::connect(net_sock_path) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
    let mut stream = stream
        .ok_or_else(|| anyhow::anyhow!("gvproxy HTTP socket not ready: {}", net_sock_path))?;

    let body = format!(r#"{{"local":":{port}","remote":"{remote_ip}:{port}"}}"#);
    let request = format!(
        "POST /services/forwarder/expose HTTP/1.0\r\nHost: unix\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body,
    );

    stream
        .write_all(request.as_bytes())
        .context("write port-forward request")?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .context("read port-forward response")?;

    if !response.contains("200") {
        anyhow::bail!("gvproxy port-forward expose failed: {}", response);
    }

    Ok(())
}

/// Start gvproxy process and wait for sockets to be ready.
pub fn start_gvproxy(
    gvproxy_bin: &Path,
    net_sock_path: &str,
    vfkit_sock_path: &str,
) -> anyhow::Result<Child> {
    // Clean up any stale sockets
    let _ = fs::remove_file(net_sock_path);
    let _ = fs::remove_file(vfkit_sock_path);

    let mut cmd = Command::new(gvproxy_bin);
    cmd.args(&[
        "--listen",
        &format!("unix://{}", net_sock_path),
        "--listen-vfkit",
        &format!("unixgram://{}", vfkit_sock_path),
        "--ssh-port",
        "-1",
    ]);

    let child = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Failed to start gvproxy")?;

    // Wait for vfkit socket to be created (indicates gvproxy is ready)
    let mut attempts = 0;
    loop {
        if Path::new(vfkit_sock_path).exists() {
            break;
        }
        if attempts > 100 {
            anyhow::bail!("Timeout waiting for gvproxy socket: {}", vfkit_sock_path);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        attempts += 1;
    }

    Ok(child)
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

    // Create a pipe for serial console input to avoid a kqueue busy-spin on macOS.
    // When the runner's check() calls wait_with_output(), it closes the subprocess's
    // stdin (fd 0). On macOS/kqueue a closed-write-end pipe fires EVFILT_READ
    // continuously, spinning the serial device at ~100% CPU.  Using a fresh pipe
    // whose write end stays open until _exit() is called prevents that.
    // libkrun takes ownership of the read fd via File::from_raw_fd(); we only
    // need to keep the write end alive, which _exit() will close for us.
    let mut pipe_fds: [libc::c_int; 2] = [-1, -1];
    if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } != 0 {
        anyhow::bail!(
            "Failed to create serial input pipe: {}",
            std::io::Error::last_os_error()
        );
    }
    let serial_read_fd = pipe_fds[0];
    // pipe_fds[1] (write end) intentionally kept open; _exit() will close it.

    unsafe {
        // FreeBSD requires a serial console; virtio console is not supported.
        // The subprocess stdout (fd 1) is piped by the runner — guest serial output appears there.
        krun_call!(krun_disable_implicit_console(ctx))?;
        krun_call!(krun_add_serial_console_default(ctx, serial_read_fd, 1))?;

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

/// Boot a FreeBSD guest with gvproxy networking enabled.
///
/// This variant:
/// - starts gvproxy process in the background
/// - adds a virtio-net device configured to use gvproxy
/// - cleans up gvproxy when test completes
pub fn setup_kernel_and_enter_with_gvproxy(
    ctx: u32,
    test_setup: TestSetup,
    assets: FreeBsdAssets,
) -> anyhow::Result<()> {
    let config_iso = create_config_iso(&test_setup.test_case, &test_setup.tmp_dir)?;

    // Socket paths are fixed (not randomised) so that check() — which runs in the
    // parent/runner process — can derive them from tmp_dir without any IPC.
    // gvproxy is started by check() in the runner process; see the note on cleanup below.
    //
    // Why gvproxy is NOT started here:
    //   krun_start_enter() terminates the subprocess via libc::_exit(), which bypasses
    //   all Rust destructors and atexit handlers.  Any Child handle created here would
    //   be leaked; gvproxy would keep running as an orphan.  Running gvproxy in the
    //   parent (runner) process and cleaning it up in check() is the correct pattern.
    let (_net_sock_str, vfkit_sock_str) = gvproxy_socket_paths(&test_setup.tmp_dir);
    let vfkit_sock_cstr =
        CString::new(vfkit_sock_str.as_bytes()).context("vfkit socket CString")?;

    let kernel_cstr =
        CString::new(assets.kernel_path.as_os_str().as_bytes()).context("kernel_path CString")?;
    let rootfs_cstr =
        CString::new(assets.iso_path.as_os_str().as_bytes()).context("rootfs iso CString")?;
    let config_iso_cstr =
        CString::new(config_iso.as_os_str().as_bytes()).context("config iso CString")?;

    // Serial input pipe — see setup_kernel_and_enter for rationale.
    let mut pipe_fds: [libc::c_int; 2] = [-1, -1];
    if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } != 0 {
        anyhow::bail!(
            "Failed to create serial input pipe: {}",
            std::io::Error::last_os_error()
        );
    }
    let serial_read_fd = pipe_fds[0];
    // pipe_fds[1] (write end) intentionally kept open; _exit() will close it.

    unsafe {
        // FreeBSD requires a serial console; virtio console is not supported.
        krun_call!(krun_disable_implicit_console(ctx))?;
        krun_call!(krun_add_serial_console_default(ctx, serial_read_fd, 1))?;

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

        // Add virtio-net device with gvproxy backend.
        // gvproxy is started by check() in the parent process; it will be listening on
        // vfkit_sock_str before krun_start_enter() actually needs the socket.
        let mac = random_mac_address();
        let mut mac_mut = mac;
        krun_call!(krun_add_net_unixgram(
            ctx,
            vfkit_sock_cstr.as_ptr(),
            -1, // use socket path, not fd
            mac_mut.as_mut_ptr(),
            COMPAT_NET_FEATURES,
            NET_FLAG_VFKIT, // indicates vfkit mode (gvproxy in vfkit compatibility)
        ))?;

        krun_call!(krun_start_enter(ctx))?;
    }
    unreachable!()
}
