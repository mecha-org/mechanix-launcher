# mechanix-launcher
Launcher for Mechanix OS

## Building for the device (aarch64)

```sh
cargo install cross
cross build --release --target aarch64-unknown-linux-gnu
```

`cross` runs the build in a container (podman or docker) built from
`cross/aarch64.Dockerfile`: Fedora 44 clang + lld against an aarch64 Fedora 44
sysroot. The first run builds the image, which takes a few minutes. The binary
lands in `target/aarch64-unknown-linux-gnu/release/launcher`.
