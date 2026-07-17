use std::env;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

fn main() {
    // Create local unversioned .so symlinks so we can link without distro -dev packages.
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let link_libs = manifest_dir.join(".link-libs");
    let _ = fs::create_dir_all(&link_libs);
    let libdir = Path::new("/usr/lib/x86_64-linux-gnu");
    for (name, soname) in [
        ("libgtk-3.so", "libgtk-3.so.0"),
        ("libgdk-3.so", "libgdk-3.so.0"),
        ("libgobject-2.0.so", "libgobject-2.0.so.0"),
        ("libglib-2.0.so", "libglib-2.0.so.0"),
        ("libcairo.so", "libcairo.so.2"),
        ("libgtk-layer-shell.so", "libgtk-layer-shell.so.0"),
        (
            "libayatana-appindicator3.so",
            "libayatana-appindicator3.so.1",
        ),
        ("libgdk_pixbuf-2.0.so", "libgdk_pixbuf-2.0.so.0"),
    ] {
        let target = libdir.join(soname);
        let link = link_libs.join(name);
        if link.exists() {
            continue;
        }
        if target.exists() {
            let _ = symlink(&target, &link);
        }
    }
    println!("cargo:rustc-link-search=native={}", link_libs.display());
    println!("cargo:rerun-if-changed=build.rs");
}
