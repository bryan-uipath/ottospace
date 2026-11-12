// screencapturekit's Swift bridge needs the system Swift runtime; its own rpath doesn't reach the
// examples or the unit tests.
fn main() {
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}
