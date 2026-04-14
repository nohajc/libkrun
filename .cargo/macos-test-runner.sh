#!/bin/bash
# Cargo target runner for macOS (aarch64-apple-darwin).
# Codesigns the test/run binary with the Hypervisor.framework entitlement
# before executing it, so that HVF-based tests can run without a developer
# certificate.
set -euo pipefail

BINARY="$1"
shift

ENTITLEMENTS=$(mktemp /tmp/hvf-test.XXXXXX)
trap 'rm -f "$ENTITLEMENTS"' EXIT

cat > "$ENTITLEMENTS" << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.hypervisor</key>
    <true/>
</dict>
</plist>
EOF

codesign --entitlements "$ENTITLEMENTS" --force -s - "$BINARY"
exec "$BINARY" "$@"
