//! Build script for the graphical interface.
//!
//! It locates the GTK4 and libadwaita development files. The environment
//! variables `PKG_CONFIG_PATH` and `PKG_CONFIG_SYSROOT_DIR` can point the
//! build at a prefix extracted from the distribution packages.

fn main() {
    pkg_config::Config::new()
        .atleast_version("4.14.0")
        .probe("gtk4")
        .expect("the GTK4 development files are required (libgtk-4-dev)");

    pkg_config::Config::new()
        .atleast_version("1.5.0")
        .probe("libadwaita-1")
        .expect("the libadwaita development files are required (libadwaita-1-dev)");

    println!("cargo:rerun-if-changed=build.rs");
}
