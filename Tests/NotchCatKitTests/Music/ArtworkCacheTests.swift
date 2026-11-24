import XCTest
@testable import NotchCatKit

final class ArtworkCacheTests: XCTestCase {
    private let bytesA = Data([0x01, 0x02, 0x03])
    private let bytesB = Data([0x0A, 0x0B])

    func testStoresDataAndFillsLaterMissingDataByIdentifier() {
        let cache = ArtworkCache()

        let withData = cache.resolve(ArtworkRef(identifier: "a", data: bytesA))
        XCTAssertEqual(withData, ArtworkRef(identifier: "a", data: bytesA))

        let filled = cache.resolve(ArtworkRef(identifier: "a", data: nil))
        XCTAssertEqual(filled, ArtworkRef(identifier: "a", data: bytesA))
    }

    func testUnknownIdentifierWithoutDataStaysEmpty() {
        let cache = ArtworkCache()
        _ = cache.resolve(ArtworkRef(identifier: "a", data: bytesA))

        let result = cache.resolve(ArtworkRef(identifier: "other", data: nil))
        XCTAssertEqual(result, ArtworkRef(identifier: "other", data: nil))
    }

    func testNewDataForSameIdentifierReplacesCachedData() {
        let cache = ArtworkCache()
        _ = cache.resolve(ArtworkRef(identifier: "a", data: bytesA))
        _ = cache.resolve(ArtworkRef(identifier: "a", data: bytesB))

        XCTAssertEqual(cache.resolve(ArtworkRef(identifier: "a", data: nil))?.data, bytesB)
    }

    func testEvictsOldestBeyondCapacity() {
        let cache = ArtworkCache(capacity: 2)
        _ = cache.resolve(ArtworkRef(identifier: "a", data: bytesA))
        _ = cache.resolve(ArtworkRef(identifier: "b", data: bytesB))
        _ = cache.resolve(ArtworkRef(identifier: "c", data: bytesA))

        XCTAssertNil(cache.resolve(ArtworkRef(identifier: "a", data: nil))?.data)
        XCTAssertEqual(cache.resolve(ArtworkRef(identifier: "b", data: nil))?.data, bytesB)
        XCTAssertEqual(cache.resolve(ArtworkRef(identifier: "c", data: nil))?.data, bytesA)
    }

    func testRefreshingAnIdentifierMakesItNewestForEviction() {
        let cache = ArtworkCache(capacity: 2)
        _ = cache.resolve(ArtworkRef(identifier: "a", data: bytesA))
        _ = cache.resolve(ArtworkRef(identifier: "b", data: bytesB))
        _ = cache.resolve(ArtworkRef(identifier: "a", data: bytesA))
        _ = cache.resolve(ArtworkRef(identifier: "c", data: bytesB))

        XCTAssertEqual(cache.resolve(ArtworkRef(identifier: "a", data: nil))?.data, bytesA)
        XCTAssertNil(cache.resolve(ArtworkRef(identifier: "b", data: nil))?.data)
    }

    func testNilRefPassesThrough() {
        XCTAssertNil(ArtworkCache().resolve(nil))
    }
}
