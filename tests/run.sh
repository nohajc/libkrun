#!/bin/sh

# This script has to be run with the working directory being "test"
# This runs the tests on the libkrun instance found by pkg-config.
# Specify PKG_CONFIG_PATH env variable to test a non-system installation of libkurn.

set -e

OS=$(uname -s)
 # macOS uses the string "arm64" but Rust uses "aarch64"
ARCH=$(uname -m | sed 's/^arm64$/aarch64/')

# Set the OS-specific library path from LIBKRUN_LIB_PATH.
# On macOS, SIP strips DYLD_LIBRARY_PATH when executing scripts via a shebang,
# so the Makefile passes it through this alternative variable instead.
# We do the same on Linux for consistency.
if [ -n "${LIBKRUN_LIB_PATH}" ]; then
	if [ "$OS" = "Darwin" ]; then
		export DYLD_LIBRARY_PATH="${LIBKRUN_LIB_PATH}:${DYLD_LIBRARY_PATH}"
	else
		export LD_LIBRARY_PATH="${LIBKRUN_LIB_PATH}:${LD_LIBRARY_PATH}"
	fi
fi 

GUEST_TARGET="${ARCH}-unknown-linux-musl"

# Run the unit tests first (this tests the testing framework itself not libkrun)
cargo test -p test_cases --features guest

# On macOS, we need to cross-compile for Linux musl
if [ "$OS" = "Darwin" ]; then
	SYSROOT="../linux-sysroot"
	if [ ! -d "$SYSROOT" ]; then
		echo "ERROR: Linux sysroot not found at $SYSROOT"
		echo "Run 'make' in the libkrun root directory first to create it."
		exit 1
	fi

	export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER="clang"
	export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C link-arg=-target -C link-arg=aarch64-linux-gnu -C link-arg=-fuse-ld=lld -C link-arg=--sysroot=$SYSROOT -C link-arg=-static"
	echo "Cross-compiling guest-agent for $GUEST_TARGET"
fi

cargo build --target=$GUEST_TARGET -p guest-agent
cargo build -p runner

# On macOS, the runner needs entitlements to use Hypervisor.framework
if [ "$OS" = "Darwin" ]; then
	codesign --entitlements ../hvf-entitlements.plist --force -s - target/debug/runner
fi

export KRUN_TEST_GUEST_AGENT_PATH="target/$GUEST_TARGET/debug/guest-agent"

# --- FreeBSD guest support ---
FREEBSD_SYSROOT="../freebsd-sysroot"
FREEBSD_INIT="../init/init-freebsd"

# Download FreeBSD kernel if KRUN_TEST_FREEBSD_KERNEL_PATH is not already set.
# The kernel binary is cached in target/freebsd-kernel/ and reused on subsequent runs.
if [ -z "${KRUN_TEST_FREEBSD_KERNEL_PATH}" ]; then
	if [ "$ARCH" = "x86_64" ]; then
		FREEBSD_KERNEL_URL="https://download.freebsd.org/releases/amd64/14.4-RELEASE/kernel.txz"
		FREEBSD_KERNEL_BIN="kernel"
	else
		FREEBSD_KERNEL_URL="https://download.freebsd.org/releases/arm64/aarch64/14.4-RELEASE/kernel.txz"
		FREEBSD_KERNEL_BIN="kernel.bin"
	fi
	FREEBSD_KERNEL_DIR="target/freebsd-kernel"
	FREEBSD_KERNEL_PATH="${FREEBSD_KERNEL_DIR}/boot/kernel/${FREEBSD_KERNEL_BIN}"
	if [ ! -f "${FREEBSD_KERNEL_PATH}" ]; then
		echo "Downloading FreeBSD 14.4-RELEASE kernel..."
		mkdir -p "${FREEBSD_KERNEL_DIR}"
		FREEBSD_KERNEL_TXZ=$(mktemp)
		if curl -fL -o "${FREEBSD_KERNEL_TXZ}" "${FREEBSD_KERNEL_URL}"; then
			tar xJf "${FREEBSD_KERNEL_TXZ}" -C "${FREEBSD_KERNEL_DIR}" \
				"./boot/kernel/${FREEBSD_KERNEL_BIN}"
			rm -f "${FREEBSD_KERNEL_TXZ}"
		else
			echo "WARNING: Failed to download FreeBSD kernel; FreeBSD tests will be skipped."
			rm -f "${FREEBSD_KERNEL_TXZ}"
		fi
	fi
	if [ -f "${FREEBSD_KERNEL_PATH}" ]; then
		export KRUN_TEST_FREEBSD_KERNEL_PATH="${FREEBSD_KERNEL_PATH}"
		echo "FreeBSD kernel: ${KRUN_TEST_FREEBSD_KERNEL_PATH}"
	fi
fi

if [ -f "${FREEBSD_SYSROOT}/.sysroot_ready" ] && [ -f "${FREEBSD_INIT}" ]; then
	FREEBSD_TARGET="${ARCH}-unknown-freebsd"
	FREEBSD_SYSROOT_ABS=$(cd "${FREEBSD_SYSROOT}" && pwd)

	if [ "$ARCH" = "x86_64" ]; then
		export CARGO_TARGET_X86_64_UNKNOWN_FREEBSD_LINKER="clang"
		export CARGO_TARGET_X86_64_UNKNOWN_FREEBSD_RUSTFLAGS="-C link-arg=-target -C link-arg=x86_64-unknown-freebsd -C link-arg=-fuse-ld=lld -C link-arg=--sysroot=${FREEBSD_SYSROOT_ABS}"
		FREEBSD_CARGO_CMD="cargo build --target=${FREEBSD_TARGET} -p guest-agent"
	else
		# aarch64-unknown-freebsd has no prebuilt stdlib in rustup; build it from source with -Z build-std.
		export CARGO_TARGET_AARCH64_UNKNOWN_FREEBSD_LINKER="clang"
		if [ "$OS" = "Darwin" ]; then
			export CARGO_TARGET_AARCH64_UNKNOWN_FREEBSD_RUSTFLAGS="-C link-arg=-target -C link-arg=aarch64-unknown-freebsd -C link-arg=-fuse-ld=lld -C link-arg=-stdlib=libc++ -C link-arg=--sysroot=${FREEBSD_SYSROOT_ABS}"
		else
			export CARGO_TARGET_AARCH64_UNKNOWN_FREEBSD_RUSTFLAGS="-C link-arg=-target -C link-arg=aarch64-unknown-freebsd -C link-arg=-fuse-ld=lld -C link-arg=--sysroot=${FREEBSD_SYSROOT_ABS}"
		fi
		FREEBSD_CARGO_CMD="cargo +nightly-2026-01-25 build -Z build-std --target=${FREEBSD_TARGET} -p guest-agent"
	fi

	echo "Cross-compiling guest-agent for ${FREEBSD_TARGET}"
	if $FREEBSD_CARGO_CMD; then
		# Build the FreeBSD test rootfs ISO: init-freebsd + FreeBSD guest-agent at the root.
		FREEBSD_ISO_STAGING=$(mktemp -d)
		cp "${FREEBSD_INIT}" "${FREEBSD_ISO_STAGING}/init-freebsd"
		cp "target/${FREEBSD_TARGET}/debug/guest-agent" "${FREEBSD_ISO_STAGING}/guest-agent"
		chmod +x "${FREEBSD_ISO_STAGING}/init-freebsd" "${FREEBSD_ISO_STAGING}/guest-agent"
		FREEBSD_ISO_PATH="target/freebsd-test-rootfs.iso"
		bsdtar cf "${FREEBSD_ISO_PATH}" --format=iso9660 -C "${FREEBSD_ISO_STAGING}" .
		rm -rf "${FREEBSD_ISO_STAGING}"
		echo "FreeBSD test rootfs ISO: ${FREEBSD_ISO_PATH}"
		export KRUN_TEST_FREEBSD_ISO_PATH="${FREEBSD_ISO_PATH}"
	else
		if [ "$ARCH" = "x86_64" ]; then
			echo "WARNING: guest-agent build for ${FREEBSD_TARGET} failed; FreeBSD tests will be skipped."
			echo "(Run: rustup target add ${FREEBSD_TARGET})"
		else
			echo "WARNING: guest-agent build for ${FREEBSD_TARGET} failed; FreeBSD tests will be skipped."
			echo "(Run: rustup toolchain install nightly-2026-01-25)"
		fi
	fi
else
	echo "FreeBSD sysroot or init/init-freebsd not found; FreeBSD tests will be skipped."
	echo "(Run 'make' with BUILD_BSD_INIT=1 in the libkrun root to build FreeBSD assets.)"
fi

# Build runner args: pass through all arguments
RUNNER_ARGS="$*"

# Add --base-dir if KRUN_TEST_BASE_DIR is set
if [ -n "${KRUN_TEST_BASE_DIR}" ]; then
	RUNNER_ARGS="${RUNNER_ARGS} --base-dir ${KRUN_TEST_BASE_DIR}"
fi

if [ "$OS" != "Darwin" ] && [ -z "${KRUN_NO_UNSHARE}" ] && which unshare 2>&1 >/dev/null; then
	unshare --user --map-root-user --net -- /bin/sh -c "ifconfig lo 127.0.0.1 && exec target/debug/runner ${RUNNER_ARGS}"
else
	echo "WARNING: Running tests without a network namespace."
	echo "Tests may fail if the required network ports are already in use."
	echo
	target/debug/runner ${RUNNER_ARGS}
fi
