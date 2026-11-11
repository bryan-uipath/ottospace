// screencapturekit's Swift bridge needs the system Swift runtime; its own rpath doesn't reach our binary.
fn main() {
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}
