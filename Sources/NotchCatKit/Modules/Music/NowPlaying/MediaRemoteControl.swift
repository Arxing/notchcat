import Foundation

/// Raw MediaRemote command codes.
enum MediaRemoteCommand: UInt32 {
    case play = 0
    case pause = 1
    case togglePlayPause = 2
    case next = 4
    case previous = 5
}

/// Sends transport commands to the system's now-playing service.
/// Exists so the rest of the module can be built and tested against a fake.
protocol MediaRemoteControlling: AnyObject {
    /// Returns `false` when the system declined the command.
    @discardableResult func send(_ command: MediaRemoteCommand) -> Bool
}

/// The ONLY file in the Swift app allowed to touch the private MediaRemote
/// framework. It is used for control only: do not add any read call here.
/// Reading now-playing state happens in the helper process, never in-process.
///
/// If Apple ever withdraws the symbol, `init?` returns nil and only this file
/// changes — the module runs without transport buttons rather than taking the
/// shell down with it.
final class MediaRemoteControl: MediaRemoteControlling {
    // Swift closures and dictionaries are not representable in C function
    // types, so the (unused) options dictionary crosses the boundary as
    // `CFDictionary`.
    private typealias SendCommandFn = @convention(c) (UInt32, CFDictionary?) -> Bool

    static let defaultFrameworkPath = "/System/Library/PrivateFrameworks/MediaRemote.framework/MediaRemote"

    private let sendCommand: SendCommandFn

    init?(frameworkPath: String = MediaRemoteControl.defaultFrameworkPath,
          symbolName: String = "MRMediaRemoteSendCommand") {
        guard let handle = dlopen(frameworkPath, RTLD_NOW),
              let symbol = dlsym(handle, symbolName)
        else { return nil }
        self.sendCommand = unsafeBitCast(symbol, to: SendCommandFn.self)
    }

    @discardableResult
    func send(_ command: MediaRemoteCommand) -> Bool {
        sendCommand(command.rawValue, nil)
    }
}
