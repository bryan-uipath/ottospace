// screencapturekit's Swift bridge needs the system Swift runtime; its own rpath doesn't reach the examples.
fn main() {
    println!("cargo:rustc-link-arg-examples=-Wl,-rpath,/usr/lib/swift");
}
