#[cfg(target_os = "freebsd")]
pub fn configure_virtio_net_ip() {
    use nix::libc;
    use std::ffi::CString;
    use std::mem;

    // ioctl constants derived from freebsd-sysroot/usr/include/sys/sockio.h
    // _IOW('i', 43, struct ifaliasreq{68}) = 0x80000000 | (68<<16) | ('i'<<8) | 43
    const SIOCAIFADDR: libc::c_ulong = 0x8044692b;
    // _IOW('i', 16, struct ifreq{32}) = 0x80000000 | (32<<16) | ('i'<<8) | 16
    const SIOCSIFFLAGS: libc::c_ulong = 0x80206910;

    // FreeBSD network structures (matching freebsd-sysroot/usr/include/net/if.h)
    #[repr(C)]
    struct sockaddr_in {
        sin_len: u8,
        sin_family: u8,
        sin_port: u16,
        sin_addr: u32,
        sin_zero: [u8; 8],
    }

    #[repr(C)]
    struct ifaliasreq {
        ifra_name: [u8; 16],
        ifra_addr: sockaddr_in,
        ifra_broadaddr: sockaddr_in,
        ifra_mask: sockaddr_in,
        ifra_ifa_vhid: i32,
    }

    // Create socket
    let sockfd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    if sockfd < 0 {
        eprintln!("Failed to create socket");
        return;
    }

    // Interface name
    let iface_name = match CString::new("vtnet0") {
        Ok(name) => name,
        Err(_) => {
            eprintln!("Failed to create interface name CString");
            return;
        }
    };
    let iface_bytes = iface_name.as_bytes();

    // Build the ifaliasreq structure
    let mut ifare: ifaliasreq = unsafe { mem::zeroed() };
    ifare.ifra_name[..iface_bytes.len()].copy_from_slice(iface_bytes);

    // Set up the address structure (192.168.127.2)
    ifare.ifra_addr = sockaddr_in {
        sin_len: mem::size_of::<sockaddr_in>() as u8,
        sin_family: libc::AF_INET as u8,
        sin_port: 0,
        sin_addr: (192u32 << 24) | (168u32 << 16) | (127u32 << 8) | 2,
        sin_zero: [0u8; 8],
    };

    // Set up the netmask (255.255.255.0)
    ifare.ifra_mask = sockaddr_in {
        sin_len: mem::size_of::<sockaddr_in>() as u8,
        sin_family: libc::AF_INET as u8,
        sin_port: 0,
        sin_addr: 0xffffff00u32,
        sin_zero: [0u8; 8],
    };

    // Set up the broadcast address (192.168.127.255)
    ifare.ifra_broadaddr = sockaddr_in {
        sin_len: mem::size_of::<sockaddr_in>() as u8,
        sin_family: libc::AF_INET as u8,
        sin_port: 0,
        sin_addr: (192u32 << 24) | (168u32 << 16) | (127u32 << 8) | 255,
        sin_zero: [0u8; 8],
    };

    // Set the interface address using ioctl
    unsafe {
        if libc::ioctl(sockfd, SIOCAIFADDR, &mut ifare as *mut _) < 0 {
            eprintln!("Failed to set IP address");
            libc::close(sockfd);
            return;
        }
    }

    // Bring the interface up
    #[repr(C)]
    struct ifreq {
        ifr_name: [u8; 16],
        ifr_union: [u8; 16],
    }

    let mut ifr: ifreq = unsafe { mem::zeroed() };
    ifr.ifr_name[..iface_bytes.len()].copy_from_slice(iface_bytes);

    // Set flags to IFF_UP
    let flags_ptr = &mut ifr.ifr_union as *mut _ as *mut u16;
    unsafe {
        *flags_ptr = libc::IFF_UP as u16;
    }

    unsafe {
        if libc::ioctl(sockfd, SIOCSIFFLAGS, &mut ifr as *mut _) < 0 {
            eprintln!("Failed to bring interface up");
        }
        libc::close(sockfd);
    }
}

#[cfg(not(target_os = "freebsd"))]
pub fn configure_virtio_net_ip() {
    // No-op on non-FreeBSD systems
}
