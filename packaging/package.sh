#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "$0")/.."

binary=target/release/bluebubbles-linux
test -x "$binary" || { echo 'Run cargo build --release --locked first.' >&2; exit 1; }
version=0.1.0
architecture=$(uname -m)
name="bluebubbles-linux-${version}-${architecture}"
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT
mkdir -p dist "$stage/$name"
install -m755 "$binary" "$stage/$name/bluebubbles-linux"
install -m644 README.md PARITY.md LICENSE NOTICE "$stage/$name/"
install -m644 packaging/app.bluebubbles.RustLinux.desktop "$stage/$name/"
install -m644 assets/icon.png "$stage/$name/app.bluebubbles.RustLinux.png"
tar -C "$stage" -czf "dist/$name.tar.gz" "$name"

if command -v dpkg-deb >/dev/null; then
    case "$architecture" in
        x86_64) debarch=amd64 ;;
        aarch64) debarch=arm64 ;;
        *) echo "Created dist/$name.tar.gz (Debian packaging skipped for $architecture)"; exit 0 ;;
    esac
    package="$stage/deb"
    mkdir -p "$package/DEBIAN" "$package/usr/bin" "$package/usr/share/applications" \
        "$package/usr/share/icons/hicolor/1024x1024/apps" "$package/usr/share/doc/bluebubbles-linux"
    install -m755 "$binary" "$package/usr/bin/bluebubbles-linux"
    install -m644 packaging/app.bluebubbles.RustLinux.desktop "$package/usr/share/applications/"
    install -m644 assets/icon.png "$package/usr/share/icons/hicolor/1024x1024/apps/app.bluebubbles.RustLinux.png"
    install -m644 README.md PARITY.md NOTICE "$package/usr/share/doc/bluebubbles-linux/"
    install -m644 LICENSE "$package/usr/share/doc/bluebubbles-linux/copyright"
    # Match the glibc requirement of the actual build, rather than claiming portability.
    glibc=$(objdump -T "$binary" | sed -n 's/.*GLIBC_\([0-9.]*\).*/\1/p' | sort -V | tail -1)
    cat > "$package/DEBIAN/control" <<EOF
Package: bluebubbles-linux
Version: $version
Architecture: $debarch
Maintainer: BlueBubbles Rust local build <noreply@localhost>
Section: net
Priority: optional
Depends: libc6 (>= $glibc), libgcc-s1, libssl3t64 | libssl3, libx11-6, libxkbcommon0, libegl1, libgl1, libwayland-client0, libxcb1
Recommends: xdg-desktop-portal, xdg-desktop-portal-gtk, dbus-user-session
Description: Native Rust Linux client for BlueBubbles
 Browse conversations and send messages through a BlueBubbles Mac server.
EOF
    dpkg-deb --root-owner-group --build "$package" "dist/bluebubbles-linux_${version}_${debarch}.deb"
fi
(cd dist && sha256sum "$name.tar.gz") > dist/SHA256SUMS
if test -n "${debarch:-}"; then
    (cd dist && sha256sum "bluebubbles-linux_${version}_${debarch}.deb") >> dist/SHA256SUMS
fi
echo "Packages written to $(pwd)/dist"
