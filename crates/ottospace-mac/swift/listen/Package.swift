// swift-tools-version:5.9
import PackageDescription

// The recogniser's shim (`listen.rs`): Apple's on-device SpeechAnalyzer behind a C ABI. Built by
// `build.rs`. Targets macOS 14: every Speech call is behind `#available(macOS 26, *)`, and with an
// SDK older than 26 (no `OTTOSPACE_SPEECH_SDK`) it builds a stub that says "unavailable".
let package = Package(
    name: "OttospaceListen",
    platforms: [.macOS("14.0")],
    products: [.library(name: "OttospaceListen", type: .static, targets: ["OttospaceListen"])],
    targets: [.target(name: "OttospaceListen", path: "Sources")]
)
