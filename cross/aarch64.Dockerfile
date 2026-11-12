# Image for `cross build --target aarch64-unknown-linux-gnu`: an x86_64
# Fedora 44 clang + lld linking against an aarch64 Fedora 44 sysroot, the
# distro the device runs. The sysroot's packages are unpacked without their
# scriptlets, so nothing is emulated.
FROM --platform=linux/amd64 registry.fedoraproject.org/fedora:44

RUN dnf -y install --setopt=install_weak_deps=False \
        clang lld llvm pkgconf-pkg-config \
 && dnf clean all

# The libraries the launcher links; GBM and EGL are not among them, since
# mecha-wayland loads both at runtime. `filesystem` goes in first and alone:
# it makes the usrmerge symlinks (/lib64, /bin…) and fails if another
# package made them as directories first.
RUN dnf -y install --installroot=/sysroot --forcearch=aarch64 \
        --releasever=44 --use-host-config --setopt=tsflags=noscripts filesystem \
 && dnf -y install --installroot=/sysroot --forcearch=aarch64 \
        --releasever=44 --use-host-config \
        --setopt=install_weak_deps=False --setopt=tsflags=noscripts \
        glibc-devel gcc libxkbcommon-devel pulseaudio-libs-devel pam-devel \
 && dnf --installroot=/sysroot clean all \
 && rm -rf /sysroot/var/cache/*

RUN printf '#!/bin/sh\nexec clang --target=aarch64-redhat-linux-gnu --sysroot=/sysroot -fuse-ld=lld "$@"\n' \
        > /usr/local/bin/aarch64-redhat-linux-clang \
 && chmod +x /usr/local/bin/aarch64-redhat-linux-clang

ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-redhat-linux-clang \
    CC_aarch64_unknown_linux_gnu=aarch64-redhat-linux-clang \
    AR_aarch64_unknown_linux_gnu=llvm-ar \
    PKG_CONFIG_ALLOW_CROSS_aarch64_unknown_linux_gnu=1 \
    PKG_CONFIG_SYSROOT_DIR_aarch64_unknown_linux_gnu=/sysroot \
    PKG_CONFIG_LIBDIR_aarch64_unknown_linux_gnu=/sysroot/usr/lib64/pkgconfig:/sysroot/usr/share/pkgconfig
