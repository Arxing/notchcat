import Combine
import Foundation

/// A running helper process.
protocol HelperProcessHandle {
    func terminate()
}

/// Starts the now-playing helper process. `onLine` receives each complete
/// output line; `onExit` receives the exit status. Throws when the process
/// cannot be started at all.
protocol HelperProcessLaunching {
    func launch(onLine: @escaping (String) -> Void, onExit: @escaping (Int32) -> Void) throws -> HelperProcessHandle
}

/// System now-playing, read by a helper process (see `PerlHelperLauncher`)
/// and decoded with `HelperMessageParser`.
///
/// An unexpected exit restarts the helper after the next entry of
/// `retryDelays`; any line received resets that sequence. Once the delays are
/// used up the source publishes nil and stays down until `start()` is called
/// again. A launch that throws counts as a failure too.
///
/// Not thread-safe: call it, and deliver the launcher's callbacks and the
/// scheduled work, on one queue (the main queue by default).
final class MediaRemoteSource: NowPlayingSource {
    let id = HelperMessageParser.sourceID

    var snapshots: AnyPublisher<NowPlayingSnapshot?, Never> { subject.eraseToAnyPublisher() }

    private let launcher: HelperProcessLaunching
    private let retryDelays: [TimeInterval]
    private let schedule: (TimeInterval, @escaping () -> Void) -> Void
    private let subject = CurrentValueSubject<NowPlayingSnapshot?, Never>(nil)

    private var isRunning = false
    private var failureCount = 0
    private var handle: HelperProcessHandle?
    /// Bumped on every launch and on `stop()`, so callbacks and scheduled
    /// restarts that belong to an earlier process are ignored.
    private var generation = 0

    init(launcher: HelperProcessLaunching,
         retryDelays: [TimeInterval] = [1, 2, 4, 8, 16, 30],
         schedule: @escaping (TimeInterval, @escaping () -> Void) -> Void = {
             DispatchQueue.main.asyncAfter(deadline: .now() + $0, execute: $1)
         }) {
        self.launcher = launcher
        self.retryDelays = retryDelays
        self.schedule = schedule
    }

    func start() {
        guard !isRunning else { return }
        isRunning = true
        failureCount = 0
        launch()
    }

    /// Terminates the helper and publishes nil, so a later `start()` does not
    /// replay this session's last snapshot.
    func stop() {
        isRunning = false
        generation += 1
        handle?.terminate()
        handle = nil
        subject.send(nil)
    }

    private func launch() {
        generation += 1
        let current = generation
        do {
            handle = try launcher.launch(
                onLine: { [weak self] line in self?.receive(line, from: current) },
                onExit: { [weak self] _ in self?.processExited(current) })
        } catch {
            handle = nil
            fail()
        }
    }

    private func receive(_ line: String, from launchGeneration: Int) {
        guard isRunning, launchGeneration == generation else { return }
        failureCount = 0
        switch HelperMessageParser.parse(line: line) {
        case .track(let snapshot)?: subject.send(snapshot)
        case .none?: subject.send(nil)
        case nil: break
        }
    }

    private func processExited(_ launchGeneration: Int) {
        guard isRunning, launchGeneration == generation else { return }
        // Output still buffered from the dead process must not be published or
        // reset the failure count, which would defeat the give-up limit.
        generation += 1
        handle = nil
        fail()
    }

    private func fail() {
        guard failureCount < retryDelays.count else {
            isRunning = false
            subject.send(nil)
            return
        }
        let delay = retryDelays[failureCount]
        failureCount += 1
        let current = generation
        schedule(delay) { [weak self] in
            guard let self, self.isRunning, current == self.generation else { return }
            self.launch()
        }
    }
}
