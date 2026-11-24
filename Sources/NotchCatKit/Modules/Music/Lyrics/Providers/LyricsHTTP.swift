import Foundation

/// How a provider's request failed, as opposed to the service answering that it
/// has nothing. `LyricsRepository` caches the two very differently, so a
/// provider must throw for these and return nil only for a clean "not found".
enum LyricsProviderError: Error, Equatable {
    /// The service answered with a non-2xx status.
    case badStatus(Int)
    /// The service answered 2xx with a body that could not be understood.
    case badResponse
}

/// Request plumbing shared by the providers.
enum LyricsHTTP {
    static let userAgent = "NotchCat"

    /// Builds a URL with every query value percent-encoded strictly. The
    /// encoding `URLComponents.queryItems` applies leaves `+` and `&` alone,
    /// and a server reads the first as a space and the second as a separator.
    static func url(_ base: String, query: [(name: String, value: String)]) -> URL {
        var components = URLComponents(string: base)!
        components.percentEncodedQueryItems = query.map {
            URLQueryItem(name: encode($0.name), value: encode($0.value))
        }
        return components.url!
    }

    private static let unreserved = CharacterSet(
        charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"
    )

    private static func encode(_ text: String) -> String {
        text.addingPercentEncoding(withAllowedCharacters: unreserved) ?? text
    }

    static func request(
        _ url: URL, timeout: TimeInterval, headers: [String: String] = [:]
    ) -> URLRequest {
        var request = URLRequest(url: url, timeoutInterval: timeout)
        request.setValue(userAgent, forHTTPHeaderField: "User-Agent")
        for (name, value) in headers {
            request.setValue(value, forHTTPHeaderField: name)
        }
        return request
    }

    /// Transport errors (including timeouts) propagate unchanged.
    static func send(
        _ request: URLRequest, using session: URLSession
    ) async throws -> (status: Int, body: Data) {
        let (body, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else { throw LyricsProviderError.badResponse }
        return (http.statusCode, body)
    }

    /// The body of a 2xx response; any other status is a failure.
    static func successBody(
        _ request: URLRequest, using session: URLSession
    ) async throws -> Data {
        let (status, body) = try await send(request, using: session)
        guard (200..<300).contains(status) else { throw LyricsProviderError.badStatus(status) }
        return body
    }
}
