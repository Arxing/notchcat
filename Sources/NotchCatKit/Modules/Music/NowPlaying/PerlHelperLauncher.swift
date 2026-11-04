import Foundation

/// Runs the `NotchCatMediaRemoteHelper` dylib inside Apple-signed `/usr/bin/perl`.
///
/// The system only hands now-playing data to Apple-signed processes, so the
/// reading happens in perl, which loads the ad-hoc-signed dylib through
/// DynaLoader and calls its entry point as an XSUB. The helper writes JSON lines
/// to stdout and exits when its stdin reaches EOF; stdin is a pipe that stays
/// open for as long as the returned handle is alive.
///
/// Callbacks are delivered on the main queue.
final class PerlHelperLauncher: HelperProcessLaunching {
    enum LaunchError: Error, Equatable {
        case perlNotFound
        case dylibNotFound
    }

    static let dylibName = "libNotchCatMediaRemoteHelper.dylib"

    /// Fixed host script. `$ARGV[0]` is the dylib's absolute path (perl's
    /// hardened runtime rejects relative paths).
    static let script = """
        use DynaLoader;
        my $lib = DynaLoader::dl_load_file($ARGV[0], 0) or die "load: " . DynaLoader::dl_error() . "\\n";
        my $sym = DynaLoader::dl_find_symbol($lib, "notchcat_helper_main") or die "symbol: " . DynaLoader::dl_error() . "\\n";
        my $xs = DynaLoader::dl_install_xsub("main::notchcat_helper_main", $sym);
        &$xs();
        """

    private let perlPath: String
    private let dylibURL: URL?
    /// The missing-dylib message is logged once per process, not once per retry.
    private static var didLogMissingDylib = false

    init(perlPath: String = "/usr/bin/perl", dylibURL: URL? = PerlHelperLauncher.defaultDylibURL()) {
        self.perlPath = perlPath
        self.dylibURL = dylibURL
    }

    /// The dylib next to the running executable, or nil when it is not there.
    static func defaultDylibURL() -> URL? {
        guard let directory = Bundle.main.executableURL?.deletingLastPathComponent() else { return nil }
        let url = directory.appendingPathComponent(dylibName)
        return FileManager.default.fileExists(atPath: url.path) ? url : nil
    }

    func launch(onLine: @escaping (String) -> Void, onExit: @escaping (Int32) -> Void) throws -> HelperProcessHandle {
        guard FileManager.default.isExecutableFile(atPath: perlPath) else { throw LaunchError.perlNotFound }
        guard let dylibURL, FileManager.default.fileExists(atPath: dylibURL.path) else {
            if !Self.didLogMissingDylib {
                Self.didLogMissingDylib = true
                NSLog("NotchCat: %@ not found (%@); system now-playing is unavailable",
                      Self.dylibName, dylibURL?.path ?? "nothing next to the executable")
            }
            throw LaunchError.dylibNotFound
        }

        let process = Process()
        let input = Pipe()
        let output = Pipe()
        process.executableURL = URL(fileURLWithPath: perlPath)
        process.arguments = ["-e", Self.script, dylibURL.standardizedFileURL.path]
        process.standardInput = input
        process.standardOutput = output

        var buffer = CappedHelperLineBuffer()
        output.fileHandleForReading.readabilityHandler = { handle in
            let data = handle.availableData
            guard !data.isEmpty else {
                handle.readabilityHandler = nil
                return
            }
            let lines = buffer.append(data)
            guard !lines.isEmpty else { return }
            DispatchQueue.main.async { lines.forEach(onLine) }
        }
        process.terminationHandler = { process in
            let status = process.terminationStatus
            DispatchQueue.main.async { onExit(status) }
        }

        do {
            try process.run()
        } catch {
            output.fileHandleForReading.readabilityHandler = nil
            throw error
        }
        return PerlHelperProcess(process: process, input: input, output: output)
    }
}

private final class PerlHelperProcess: HelperProcessHandle {
    private let process: Process
    /// Held so the helper's stdin stays open; releasing it closes the pipe and
    /// the helper exits on EOF.
    private let input: Pipe
    private let output: Pipe

    init(process: Process, input: Pipe, output: Pipe) {
        self.process = process
        self.input = input
        self.output = output
    }

    func terminate() {
        output.fileHandleForReading.readabilityHandler = nil
        try? input.fileHandleForWriting.close()
        if process.isRunning { process.terminate() }
    }

    deinit {
        terminate()
    }
}

/// `HelperLineBuffer` with a bound on how long one line may grow. A line longer
/// than `maxLineBytes` (counted without its newline) is discarded up to and
/// including its terminating newline, so a misbehaving helper cannot make the
/// pending bytes grow without limit.
struct CappedHelperLineBuffer {
    static let defaultMaxLineBytes = 4 * 1024 * 1024

    private let maxLineBytes: Int
    private var lines = HelperLineBuffer()
    private var pendingCount = 0
    private var isDiscarding = false

    init(maxLineBytes: Int = CappedHelperLineBuffer.defaultMaxLineBytes) {
        self.maxLineBytes = maxLineBytes
    }

    mutating func append(_ data: Data) -> [String] {
        var result: [String] = []
        var rest = data[...]
        while !rest.isEmpty {
            let newline = rest.firstIndex(of: 0x0A)
            let segmentEnd = newline ?? rest.endIndex
            let segmentLength = segmentEnd - rest.startIndex
            if isDiscarding {
                // Dropped.
            } else if pendingCount + segmentLength > maxLineBytes {
                lines = HelperLineBuffer()
                pendingCount = 0
                isDiscarding = true
            } else {
                let chunkEnd = newline.map { $0 + 1 } ?? rest.endIndex
                result += lines.append(Data(rest[rest.startIndex..<chunkEnd]))
                pendingCount += segmentLength
            }
            guard let newline else { break }
            isDiscarding = false
            pendingCount = 0
            rest = rest[(newline + 1)...]
        }
        return result
    }
}
