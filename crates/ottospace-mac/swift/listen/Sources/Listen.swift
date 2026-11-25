// Speech to text on this Mac: one SpeechAnalyzer with a SpeechTranscriber per stream, fed 16 kHz
// mono Float32 samples, results by callback. On device only: SpeechAnalyzer's transcriber sends
// no audio anywhere, and its model is the system's (AssetInventory), installed by the system.
// No Speech prompt: Apple's "Asking Permission to Use Speech Recognition" says the permission
// process "only applies to speech recognition using SFSpeechRecognizer".

import AVFoundation
import Foundation
#if OTTOSPACE_SPEECH_SDK
import Speech
#endif

/// A result: `kind` 0 a line (volatile or final), 1 the stream's error, 2 it finished.
public typealias ListenCallback = @convention(c) (
    UnsafeMutableRawPointer?, Int32, UnsafePointer<CChar>?, UInt64, UInt64, Bool
) -> Void

/// A done-callback: `ok`, and why not.
public typealias ListenDone = @convention(c) (UnsafeMutableRawPointer?, Bool, UnsafePointer<CChar>?) -> Void

/// Runs `body` on a task and waits for it: for the C ABI's calls that answer at once. Called
/// only off the main thread.
private func blocking<T>(_ body: @escaping @Sendable () async -> T) -> T {
    let done = DispatchSemaphore(value: 0)
    var out: T?
    Task.detached {
        out = await body()
        done.signal()
    }
    done.wait()
    return out!
}

private func say(_ text: String, _ f: (UnsafePointer<CChar>) -> Void) {
    text.withCString(f)
}

/// The name given, as Swift holds it: a C string does not cross into a task.
private func named(_ name: UnsafePointer<CChar>?) -> String? {
    name.map { String(cString: $0) }
}

#if OTTOSPACE_SPEECH_SDK
/// The locale to transcribe that the recogniser supports: `name` (a BCP 47 identifier) if given;
/// else the system's, then each of the user's preferred languages in turn (a system locale such
/// as `en_US_POSIX` matches none).
@available(macOS 26, *)
private func chosen(_ name: String?) async -> Locale? {
    if let name {
        return await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: name))
    }
    for want in [Locale.current] + Locale.preferredLanguages.map({ Locale(identifier: $0) }) {
        if let locale = await SpeechTranscriber.supportedLocale(equivalentTo: want) {
            return locale
        }
    }
    return nil
}

/// `locale` reserved for this app: until it is, the system says its model is only `supported`,
/// even with it on disk, and installs it for no one. Idempotent; at most 5 per app.
@available(macOS 26, *)
private func reserve(_ locale: Locale) async {
    _ = try? await AssetInventory.reserve(locale: locale)
}
#endif

/// 0: no recogniser here (below macOS 26, or built without its SDK). 1: supported, its model not
/// installed. 2: installed. 3: being downloaded. 4: this locale is not supported. 5: macOS 26, but
/// this Mac cannot run the transcriber (`isAvailable` false: a virtual machine, for one).
@_cdecl("ottospace_listen_status")
public func ottospace_listen_status(_ locale: UnsafePointer<CChar>?) -> Int32 {
    #if OTTOSPACE_SPEECH_SDK
    if #available(macOS 26, *) {
        let name = named(locale)
        return blocking {
            guard SpeechTranscriber.isAvailable else { return 5 }
            guard let locale = await chosen(name) else { return 4 }
            let module = SpeechTranscriber(locale: locale, preset: .timeIndexedProgressiveTranscription)
            switch await AssetInventory.status(forModules: [module]) {
            case .installed: return 2
            case .downloading: return 3
            case .supported: return 1
            default: return 4
            }
        }
    }
    #endif
    return 0
}

/// Asks the system to download and install the model for `locale`; `done` when it is in, or why
/// not. The system fetches Apple's model into its own store, shared with other apps.
@_cdecl("ottospace_listen_install")
public func ottospace_listen_install(
    _ locale: UnsafePointer<CChar>?, _ done: ListenDone, _ context: UnsafeMutableRawPointer?
) {
    let raw = UInt(bitPattern: context)
    #if OTTOSPACE_SPEECH_SDK
    if #available(macOS 26, *) {
        let name = named(locale)
        Task.detached {
            let context = UnsafeMutableRawPointer(bitPattern: raw)
            do {
                guard let locale = await chosen(name) else {
                    return say("this language is not supported") { done(context, false, $0) }
                }
                await reserve(locale)
                let module = SpeechTranscriber(locale: locale, preset: .timeIndexedProgressiveTranscription)
                if let request = try await AssetInventory.assetInstallationRequest(supporting: [module]) {
                    try await request.downloadAndInstall()
                }
                done(context, true, nil)
            } catch {
                say(error.localizedDescription) { done(context, false, $0) }
            }
        }
        return
    }
    #endif
    say("needs macOS 26") { done(UnsafeMutableRawPointer(bitPattern: raw), false, $0) }
}

#if OTTOSPACE_SPEECH_SDK
/// One stream's recogniser.
@available(macOS 26, *)
final class Recogniser: @unchecked Sendable {
    let input: AsyncStream<AnalyzerInput>.Continuation
    let converter: AVAudioConverter?
    let source: AVAudioFormat
    let target: AVAudioFormat
    let lock = NSLock()

    init(input: AsyncStream<AnalyzerInput>.Continuation, source: AVAudioFormat, target: AVAudioFormat) {
        self.input = input
        self.source = source
        self.target = target
        converter = source == target ? nil : AVAudioConverter(from: source, to: target)
    }

    /// `n` samples at the source rate, converted and handed to the analyzer.
    func feed(_ samples: UnsafePointer<Float>, _ n: Int) {
        lock.lock()
        defer { lock.unlock() }
        guard n > 0, let buffer = AVAudioPCMBuffer(pcmFormat: source, frameCapacity: AVAudioFrameCount(n)),
            let channel = buffer.floatChannelData
        else { return }
        buffer.frameLength = AVAudioFrameCount(n)
        channel[0].update(from: samples, count: n)
        var out = buffer
        if let converter {
            let ratio = target.sampleRate / source.sampleRate
            let capacity = AVAudioFrameCount(Double(n) * ratio) + 32
            guard let converted = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: capacity) else { return }
            var given = false
            var error: NSError?
            converter.convert(to: converted, error: &error) { _, status in
                if given {
                    status.pointee = .noDataNow
                    return nil
                }
                given = true
                status.pointee = .haveData
                return buffer
            }
            if error != nil { return }
            out = converted
        }
        input.yield(AnalyzerInput(buffer: out))
    }
}
#endif

/// A new recogniser for `locale` (or the system's), its results to `callback` with `context`, on a
/// thread of its own: `kind` 0 a line (`text`, `start_ms`, `end_ms`, `final`), 1 an error (`text`),
/// 2 finished. Null: no recogniser here, or its model is not installed.
/// Null with `why` set when it cannot start. 1: no recogniser here (below macOS 26, or built
/// without its SDK). 2: no supported language. 3, 4, 5: the model is supported but not installed,
/// downloading, or unsupported. 6: no audio format the transcriber takes. 7: `sampleRate` is no format.
@_cdecl("ottospace_listen_new")
public func ottospace_listen_new(
    _ locale: UnsafePointer<CChar>?, _ sampleRate: Double, _ callback: ListenCallback,
    _ context: UnsafeMutableRawPointer?, _ why: UnsafeMutablePointer<Int32>?
) -> UnsafeMutableRawPointer? {
    why?.pointee = 1
    #if OTTOSPACE_SPEECH_SDK
    if #available(macOS 26, *) {
        let name = named(locale)
        let raw = UInt(bitPattern: context)
        let (made, failed): (Recogniser?, Int32) = blocking {
            guard let locale = await chosen(name) else { return (nil, 2) }
            await reserve(locale)
            let transcriber = SpeechTranscriber(locale: locale, preset: .timeIndexedProgressiveTranscription)
            switch await AssetInventory.status(forModules: [transcriber]) {
            case .installed: break
            case .supported: return (nil, 3)
            case .downloading: return (nil, 4)
            default: return (nil, 5)
            }
            guard let target = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [transcriber]) else {
                return (nil, 6)
            }
            guard
                let source = AVAudioFormat(
                    commonFormat: .pcmFormatFloat32, sampleRate: sampleRate, channels: 1, interleaved: false)
            else { return (nil, 7) }
            let (stream, input) = AsyncStream<AnalyzerInput>.makeStream()
            let analyzer = SpeechAnalyzer(modules: [transcriber])
            let recogniser = Recogniser(input: input, source: source, target: target)
            Task.detached {
                let context = UnsafeMutableRawPointer(bitPattern: raw)
                let results = Task {
                    do {
                        for try await result in transcriber.results {
                            let text = String(result.text.characters)
                            let start = UInt64(max(0, result.range.start.seconds * 1000))
                            let end = UInt64(max(0, result.range.end.seconds * 1000))
                            say(text) { callback(context, 0, $0, start, end, result.isFinal) }
                        }
                    } catch {
                        say(error.localizedDescription) { callback(context, 1, $0, 0, 0, false) }
                    }
                }
                do {
                    // Until `finish` ends the input; then what is volatile is made final.
                    if let last = try await analyzer.analyzeSequence(stream) {
                        try await analyzer.finalizeAndFinish(through: last)
                    } else {
                        await analyzer.cancelAndFinishNow()
                    }
                } catch {
                    // Finished all the same, so its results end and the last callback comes.
                    await analyzer.cancelAndFinishNow()
                    say(error.localizedDescription) { callback(context, 1, $0, 0, 0, false) }
                }
                await results.value
                callback(context, 2, nil, 0, 0, true)
            }
            return (recogniser, 0)
        }
        why?.pointee = failed
        // Kept for `finish` to let go of. The analyzer finishes when its input does.
        guard let made else { return nil }
        return Unmanaged.passRetained(Box(made)).toOpaque()
    }
    #endif
    return nil
}

/// Holds a recogniser for the C side.
final class Box {
    let value: AnyObject
    init(_ value: AnyObject) { self.value = value }
}

/// `n` samples, Float32 at the rate given to `new`.
@_cdecl("ottospace_listen_feed")
public func ottospace_listen_feed(_ handle: UnsafeMutableRawPointer?, _ samples: UnsafePointer<Float>?, _ n: Int) {
    #if OTTOSPACE_SPEECH_SDK
    if #available(macOS 26, *), let handle, let samples,
        let recogniser = Unmanaged<Box>.fromOpaque(handle).takeUnretainedValue().value as? Recogniser
    {
        recogniser.feed(samples, n)
    }
    #endif
}

/// No more samples: the last lines are made final, then the callback's `kind` 2. The handle is
/// let go of; its callback's context must live until then.
@_cdecl("ottospace_listen_finish")
public func ottospace_listen_finish(_ handle: UnsafeMutableRawPointer?) {
    guard let handle else { return }
    let box = Unmanaged<Box>.fromOpaque(handle).takeRetainedValue()
    #if OTTOSPACE_SPEECH_SDK
    if #available(macOS 26, *), let recogniser = box.value as? Recogniser {
        recogniser.input.finish()
    }
    #endif
    _ = box
}
