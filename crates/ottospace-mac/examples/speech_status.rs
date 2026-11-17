//! Says whether this Mac's on-device recogniser can transcribe a language, as the shim sees it:
//! absent, unsupported, not installed, downloading or installed. It hears nothing. With
//! `--install` it asks the system to download Apple's model first (CI's app-window job does, for
//! the canary's transcript test); without it, nothing is downloaded.
//!
//! `cargo run -p ottospace-mac --example speech_status -- [--install] [locale]`

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let install = args.iter().any(|arg| arg == "--install");
    let locale = args.iter().find(|arg| *arg != "--install").cloned();
    if install && ottospace_mac::speech::status(locale.as_deref()) == ottospace_mac::speech::Status::NotInstalled {
        let (send, receive) = std::sync::mpsc::channel();
        ottospace_mac::speech::install(locale.as_deref(), move |done| {
            let _ = send.send(done);
        });
        println!("install: {:?}", receive.recv());
    }
    println!("{:?}", ottospace_mac::speech::status(locale.as_deref()));
}
