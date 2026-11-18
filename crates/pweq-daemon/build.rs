//! Build script for the daemon.
//!
//! It locates the PipeWire development files. On a system where they are
//! installed system wide this is a no-op; the environment variables
//! `PKG_CONFIG_PATH` and `PKG_CONFIG_SYSROOT_DIR` can point the build at a
//! prefix extracted from the distribution packages.
//!
//! Only `libpipewire-0.3` is probed: its pkg-config file requires
//! `libspa-0.2`, so the SPA headers come along with it. The SPA library
//! itself is not a separate shared object, it lives inside libpipewire.

fn main() {
    pkg_config::Config::new()
        .atleast_version("1.0.0")
        .probe("libpipewire-0.3")
        .expect(
            "the PipeWire development files are required (libpipewire-0.3-dev); \
             install them or point PKG_CONFIG_PATH at a prefix that has them",
        );

    println!("cargo:rerun-if-changed=build.rs");
}
