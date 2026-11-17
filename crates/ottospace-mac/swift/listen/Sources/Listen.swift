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

/// The locale to transcribe: `named` (a BCP 47 identifier) or the system's.
private func wanted(_ named: UnsafePointer<CChar>?) -> Locale {
    named.map { Locale(identifier: String(cString: $0)) } ?? Locale.current
}

/// 0: no recogniser here (below macOS 26, or built without its SDK). 1: supported, its model not
/// installed. 2: installed. 3: being downloaded. 4: this locale is not supported.
@_cdecl("ottospace_listen_status")
public func ottospace_listen_status(_ locale: UnsafePointer<CChar>?) -> Int32 {
    #if OTTOSPACE_SPEECH_SDK
    if #available(macOS 26, *) {
        let want = wanted(locale)
        return blocking {
            guard SpeechTranscriber.isAvailable,
                let locale = await SpeechTranscriber.supportedLocale(equivalentTo: want)
            else { return 4 }
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
        let want = wanted(locale)
        Task.detached {
            let context = UnsafeMutableRawPointer(bitPattern: raw)
            do {
                guard let locale = await SpeechTranscriber.supportedLocale(equivalentTo: want) else {
                    return say("this language is not supported") { done(context, false, $0) }
                }
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
    var task: Task<Void, Never>?
    var fed: Int64 = 0

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
        fed += Int64(out.frameLength)
        input.yield(AnalyzerInput(buffer: out))
    }
}
#endif

/// A new recogniser for `locale` (or the system's), its results to `callback` with `context`, on a
/// thread of its own: `kind` 0 a line (`text`, `start_ms`, `end_ms`, `final`), 1 an error (`text`),
/// 2 finished. Null: no recogniser here, or its model is not installed.
@_cdecl("ottospace_listen_new")
public func ottospace_listen_new(
    _ locale: UnsafePointer<CChar>?, _ sampleRate: Double, _ callback: ListenCallback,
    _ context: UnsafeMutableRawPointer?
) -> UnsafeMutableRawPointer? {
    #if OTTOSPACE_SPEECH_SDK
    if #available(macOS 26, *) {
        let want = wanted(locale)
        let raw = UInt(bitPattern: context)
        let made: Recogniser? = blocking {
            guard let locale = await SpeechTranscriber.supportedLocale(equivalentTo: want) else { return nil }
            let transcriber = SpeechTranscriber(locale: locale, preset: .timeIndexedProgressiveTranscription)
            guard await AssetInventory.status(forModules: [transcriber]) == .installed,
                let target = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [transcriber]),
                let source = AVAudioFormat(
                    commonFormat: .pcmFormatFloat32, sampleRate: sampleRate, channels: 1, interleaved: false)
            else { return nil }
            let (stream, input) = AsyncStream<AnalyzerInput>.makeStream()
            let analyzer = SpeechAnalyzer(modules: [transcriber])
            let recogniser = Recogniser(input: input, source: source, target: target)
            recogniser.task = Task.detached {
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
            return recogniser
        }
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
