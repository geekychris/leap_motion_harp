use std::env;
use std::path::PathBuf;

/// Locate the LeapSDK lib directory. Honours $LEAP_SDK_DIR if set, otherwise
/// falls back to the platform-default Ultraleap install path. The resulting
/// directory is added as a link-search dir, libLeapC is linked, and the same
/// path is baked into the binary as an absolute rpath so it Just Works at
/// runtime without DYLD_LIBRARY_PATH / LD_LIBRARY_PATH.
fn main() {
    println!("cargo:rerun-if-env-changed=LEAP_SDK_DIR");

    let sdk_lib = locate_sdk_lib().unwrap_or_else(|| {
        eprintln!(
            "
error: could not locate the LeapSDK lib directory.

Set the LEAP_SDK_DIR environment variable to the directory containing
libLeapC.dylib (macOS), libLeapC.so (Linux), or LeapC.lib (Windows).

Default search paths checked:
  macOS:   /Applications/Ultraleap Hand Tracking.app/Contents/LeapSDK/lib
  Linux:   /usr/lib/ultraleap-hand-tracking-service
           /usr/lib
  Windows: C:\\Program Files\\Ultraleap\\LeapSDK\\lib\\x64

Install Ultraleap Hand Tracking from https://leap2.ultraleap.com/downloads/
and re-run the build.
"
        );
        std::process::exit(1);
    });

    let sdk_lib_str = sdk_lib.to_string_lossy();
    println!("cargo:rustc-link-search=native={sdk_lib_str}");
    println!("cargo:rustc-link-lib=dylib=LeapC");

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    println!("cargo:rustc-link-arg=-Wl,-rpath,{sdk_lib_str}");
}

fn locate_sdk_lib() -> Option<PathBuf> {
    if let Ok(p) = env::var("LEAP_SDK_DIR") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    for candidate in default_candidates() {
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn default_candidates() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        vec![PathBuf::from(
            "/Applications/Ultraleap Hand Tracking.app/Contents/LeapSDK/lib",
        )]
    }
    #[cfg(target_os = "linux")]
    {
        vec![
            PathBuf::from("/usr/lib/ultraleap-hand-tracking-service"),
            PathBuf::from("/usr/lib"),
        ]
    }
    #[cfg(target_os = "windows")]
    {
        vec![PathBuf::from(
            "C:\\Program Files\\Ultraleap\\LeapSDK\\lib\\x64",
        )]
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Vec::new()
    }
}
