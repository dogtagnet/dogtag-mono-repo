import Foundation

/// Filesystem removal for DogTag's device-local data files, deliberately Foundation-only so the
/// host-less unit-test bundle can cover it (see `project.yml`).
///
/// Why a dedicated sweeper rather than a bare `removeItem`: `ProfileTreeStore.write` stages every
/// update through sibling `.<name>.<token>.tmp` / `.<name>.<token>.bak` files that hold the SAME
/// owner-secret material as the destination. Both are cleaned up on the happy path, but a crash or a
/// mid-write failure can leave one behind. A "delete my dog tags" action that removed only the
/// canonical file would leave a readable copy of the very secret it promised to destroy, so the sweep
/// takes the staging siblings too.
enum LocalDataSweep {
    struct Failure {
        let name: String
        let error: Error
    }

    /// What a sweep actually did. `failures` non-empty means the caller MUST report a partial result
    /// rather than claiming success - a destructive action that silently half-ran is worse than one
    /// that says so.
    struct Outcome {
        var removed: [String] = []
        var failures: [Failure] = []

        var isComplete: Bool { failures.isEmpty }

        var failureSummary: String? {
            guard !failures.isEmpty else { return nil }
            return failures
                .map { "\($0.name) (\($0.error.localizedDescription))" }
                .joined(separator: ", ")
        }

        mutating func merge(_ other: Outcome) {
            removed.append(contentsOf: other.removed)
            failures.append(contentsOf: other.failures)
        }
    }

    /// Remove the named `files`, their staging siblings, and the named `directories` (recursively)
    /// from `directory`.
    ///
    /// Absent entries are not failures: the sweep is idempotent, so re-running it over an
    /// already-clean directory succeeds and reports nothing removed.
    static func remove(
        from directory: URL,
        files: [String] = [],
        directories: [String] = [],
        manager: FileManager = .default
    ) -> Outcome {
        var outcome = Outcome()
        let targets = files + stagingSiblings(in: directory, of: files, manager: manager) + directories
        for name in targets {
            let url = directory.appendingPathComponent(name)
            guard manager.fileExists(atPath: url.path) else { continue }
            do {
                try manager.removeItem(at: url)
                outcome.removed.append(name)
            } catch {
                outcome.failures.append(Failure(name: name, error: error))
            }
        }
        return outcome
    }

    /// The `.<file>.<token>.tmp` / `.<file>.<token>.bak` leftovers `ProfileTreeStore.write` stages
    /// beside each file. Matched by prefix rather than suffix so a partially-written stage file with
    /// any extension is still caught.
    static func stagingSiblings(
        in directory: URL,
        of files: [String],
        manager: FileManager = .default
    ) -> [String] {
        guard !files.isEmpty,
              let entries = try? manager.contentsOfDirectory(atPath: directory.path) else { return [] }
        let prefixes = files.map { ".\($0)." }
        return entries
            .filter { entry in prefixes.contains { entry.hasPrefix($0) } }
            .sorted()
    }
}
