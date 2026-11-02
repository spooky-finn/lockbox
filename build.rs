use std::{env, path::PathBuf, process::Command};

fn main() {
    const SOURCE: &str = "src/platform/touchid/secure_enclave.swift";
    println!("cargo:rerun-if-changed={SOURCE}");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
    println!("cargo:rerun-if-env-changed=SDKROOT");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        arch => panic!("unsupported macOS architecture: {arch}"),
    };
    let minimum = env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "11.0".into());
    let target = format!("{arch}-apple-macosx{minimum}");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());

    // CryptoKit exposes the supported, encrypted Secure Enclave key representation.
    // Compile only on macOS; no signing identity or provisioning profile is needed.
    let status = Command::new("xcrun")
        .args(["swiftc", "-target", &target, "-O", "-parse-as-library"])
        .args(["-emit-library", "-static", "-module-name", "LockboxTouchID"])
        .arg("-module-cache-path")
        .arg(out.join("swift-cache"))
        .arg(SOURCE)
        .arg("-o")
        .arg(out.join("liblockbox_touch_id.a"))
        .status()
        .expect("Touch ID requires the Swift compiler from Xcode Command Line Tools");
    assert!(
        status.success(),
        "could not compile the Touch ID CryptoKit bridge"
    );

    let info = Command::new("xcrun")
        .args(["swiftc", "-target", &target, "-print-target-info"])
        .output()
        .expect("could not locate the Swift runtime");
    assert!(info.status.success(), "could not locate the Swift runtime");
    let info: serde_json::Value = serde_json::from_slice(&info.stdout).unwrap();
    for path in info["paths"]["runtimeLibraryPaths"].as_array().unwrap() {
        println!("cargo:rustc-link-search=native={}", path.as_str().unwrap());
    }
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=lockbox_touch_id");
}
