use std::env;
use std::process::Command;

// screencapturekit's Swift bridge needs the system Swift runtime; its own rpath doesn't reach the
// examples or the unit tests.
//
// The recogniser's shim (`swift/listen`, `speech.rs`), built here as the `screencapturekit` crate
// builds its bridge: `swift build` into OUT_DIR, linked static. With an SDK of 26 or later it is
// built with `OTTOSPACE_SPEECH_SDK`; with an older one it is a stub that says "unavailable". The
// binary still launches on macOS 14: Speech.framework is there (SFSpeechRecognizer), and the
// macOS 26 symbols are weak because every call is behind `#available`.
fn main() {
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    println!("cargo:rerun-if-changed=swift/listen");
    println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
    println!("cargo:rerun-if-env-changed=SDKROOT");
    let sdk = Command::new("xcrun")
        .args(["--sdk", "macosx", "--show-sdk-version"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8_lossy(&out.stdout).trim().split('.').next()?.parse::<u32>().ok());
    let speech = sdk.is_some_and(|major| major >= 26);
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x86_64",
        _ => "arm64",
    };
    let out = env::var("OUT_DIR").expect("OUT_DIR");
    let scratch = format!("{out}/listen-{arch}-{}", if speech { "speech" } else { "stub" });
    let triple = format!("{arch}-apple-macosx14.0");
    let mut args = vec!["build", "-c", "release", "--triple", &triple, "--package-path", "swift/listen"];
    args.extend(["--scratch-path", &scratch]);
    if speech {
        args.extend(["-Xswiftc", "-DOTTOSPACE_SPEECH_SDK"]);
    }
    let built = Command::new("swift").args(&args).output().expect("swift build of swift/listen");
    assert!(
        built.status.success(),
        "swift build of swift/listen failed:\n{}\n{}",
        String::from_utf8_lossy(&built.stdout),
        String::from_utf8_lossy(&built.stderr)
    );
    let bin = Command::new("swift").args(&args).arg("--show-bin-path").output().expect("swift --show-bin-path");
    let bin = String::from_utf8_lossy(&bin.stdout).trim().to_string();
    println!("cargo:rustc-link-search=native={bin}");
    println!("cargo:rustc-link-lib=static=OttospaceListen");
    println!("cargo:rustc-link-lib=framework=AVFoundation");
}
