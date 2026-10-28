import Foundation

/// Raw MediaRemote command codes.
enum MediaRemoteCommand: UInt32 {
    case play = 0
    case pause = 1
    case togglePlayPause = 2
    case next = 4
    case previous = 5
}

/// Everything the Music module needs from the system's now-playing service.
/// Exists so the rest of the module can be built and tested against a fake.
protocol MediaRemoteProviding: AnyObject {
    /// The current now-playing metadata, or `nil` when nothing is playing
    /// (or the system did not answer in time).
    func nowPlayingInfo() async -> [String: Any]?
    func isPlaying() async -> Bool
    /// Sends a transport command. Returns `false` when the control symbol is
    /// unavailable on this system or the system declined the command.
    @discardableResult func send(_ command: MediaRemoteCommand) -> Bool

    /// Starts reporting now-playing changes. `onChange` may be invoked on any
    /// thread, and may fire more than once per real change. Calling this again
    /// replaces the previous callback. Silent if the system offers no
    /// notifications — callers are expected to also poll as a safety net.
    func startObserving(_ onChange: @escaping () -> Void)
    func stopObserving()
}

/// The ONLY file in this project allowed to touch the private MediaRemote
/// framework. Everything downstream consumes plain value types, so if Apple
/// ever withdraws these symbols only this file changes — and the module
/// disables itself rather than taking the shell down with it.
///
/// Reading (now-playing info, play state) is REQUIRED: without it `init?`
/// returns nil. Control and change notifications are OPTIONAL: losing them
/// degrades the module but never takes down the read-only display.
final class MediaRemoteBridge: MediaRemoteProviding {
    // Swift closures and dictionaries are not representable in C function
    // types, so completion handlers are `@convention(block)` and dictionaries
    // cross the boundary as `CFDictionary`.
    private typealias GetInfoFn = @convention(c) (DispatchQueue, @escaping @convention(block) (CFDictionary?) -> Void) -> Void
    private typealias GetIsPlayingFn = @convention(c) (DispatchQueue, @escaping @convention(block) (Bool) -> Void) -> Void
    private typealias SendCommandFn = @convention(c) (UInt32, CFDictionary?) -> Bool
    private typealias RegisterFn = @convention(c) (DispatchQueue) -> Void
    private typealias UnregisterFn = @convention(c) () -> Void

    /// Symbol names, injectable so tests can simulate a withdrawn symbol.
    struct SymbolNames {
        var getNowPlayingInfo = "MRMediaRemoteGetNowPlayingInfo"
        var getIsPlaying = "MRMediaRemoteGetNowPlayingApplicationIsPlaying"
        var sendCommand = "MRMediaRemoteSendCommand"
        var registerForNotifications = "MRMediaRemoteRegisterForNowPlayingNotifications"
        var unregisterForNotifications = "MRMediaRemoteUnregisterForNowPlayingNotifications"
    }

    static let defaultFrameworkPath = "/System/Library/PrivateFrameworks/MediaRemote.framework/MediaRemote"

    /// Posted on `NotificationCenter.default` once registered for notifications.
    static let changeNotificationNames: [Notification.Name] = [
        Notification.Name("kMRMediaRemoteNowPlayingInfoDidChangeNotification"),
        Notification.Name("kMRMediaRemoteNowPlayingApplicationIsPlayingDidChangeNotification"),
    ]

    /// How long to wait for the system before giving up on a read.
    private static let readTimeout: TimeInterval = 5

    // Required.
    private let getInfo: GetInfoFn
    private let getIsPlaying: GetIsPlayingFn
    // Optional.
    private let sendCommand: SendCommandFn?
    private let registerForNotifications: RegisterFn?
    private let unregisterForNotifications: UnregisterFn?

    private let queue = DispatchQueue(label: "com.notchcat.mediaremote")
    private let observerLock = NSLock()
    private var observers: [NSObjectProtocol] = []

    init?(frameworkPath: String = MediaRemoteBridge.defaultFrameworkPath,
          symbols: SymbolNames = SymbolNames()) {
        guard let handle = dlopen(frameworkPath, RTLD_NOW),
              let info = dlsym(handle, symbols.getNowPlayingInfo),
              let playing = dlsym(handle, symbols.getIsPlaying)
        else { return nil }

        self.getInfo = unsafeBitCast(info, to: GetInfoFn.self)
        self.getIsPlaying = unsafeBitCast(playing, to: GetIsPlayingFn.self)
        self.sendCommand = dlsym(handle, symbols.sendCommand)
            .map { unsafeBitCast($0, to: SendCommandFn.self) }
        self.registerForNotifications = dlsym(handle, symbols.registerForNotifications)
            .map { unsafeBitCast($0, to: RegisterFn.self) }
        self.unregisterForNotifications = dlsym(handle, symbols.unregisterForNotifications)
            .map { unsafeBitCast($0, to: UnregisterFn.self) }
    }

    deinit { stopObserving() }

    func nowPlayingInfo() async -> [String: Any]? {
        await withCheckedContinuation { continuation in
            let once = ResumeOnce(continuation)
            getInfo(queue) { info in
                let dict = (info as? [String: Any]) ?? [:]
                once.resume(returning: dict.isEmpty ? nil : dict)
            }
            queue.asyncAfter(deadline: .now() + Self.readTimeout) { once.resume(returning: nil) }
        }
    }

    func isPlaying() async -> Bool {
        await withCheckedContinuation { continuation in
            let once = ResumeOnce(continuation)
            getIsPlaying(queue) { playing in once.resume(returning: playing) }
            queue.asyncAfter(deadline: .now() + Self.readTimeout) { once.resume(returning: false) }
        }
    }

    @discardableResult
    func send(_ command: MediaRemoteCommand) -> Bool {
        guard let sendCommand else { return false }
        return sendCommand(command.rawValue, nil)
    }

    func startObserving(_ onChange: @escaping () -> Void) {
        stopObserving()
        registerForNotifications?(queue)
        let center = NotificationCenter.default
        let tokens = Self.changeNotificationNames.map { name in
            center.addObserver(forName: name, object: nil, queue: nil) { _ in onChange() }
        }
        observerLock.lock()
        observers = tokens
        observerLock.unlock()
    }

    func stopObserving() {
        observerLock.lock()
        let tokens = observers
        observers = []
        observerLock.unlock()

        guard !tokens.isEmpty else { return }
        tokens.forEach { NotificationCenter.default.removeObserver($0) }
        unregisterForNotifications?()
    }
}

/// Resumes a continuation at most once, whichever of the system callback or
/// the timeout arrives first.
private final class ResumeOnce<T>: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<T, Never>?

    init(_ continuation: CheckedContinuation<T, Never>) { self.continuation = continuation }

    func resume(returning value: T) {
        lock.lock()
        let c = continuation
        continuation = nil
        lock.unlock()
        c?.resume(returning: value)
    }
}
