import Foundation

/// A dynamically-typed JSON value: the Swift analog of `serde_json::Value`.
///
/// The Rust wire structs carry free-form `metadata`, `params`, and `provenance`
/// maps (`BTreeMap<String, serde_json::Value>`). Swift `Codable` has no built-in
/// "any JSON" type, so this enum models it explicitly and round-trips losslessly.
///
/// `Sendable` so the wire structs that embed it satisfy Swift 6 strict
/// concurrency without `@unchecked`.
public enum JSONValue: Codable, Hashable, Sendable {
    case string(String)
    case number(Double)
    case bool(Bool)
    case object([String: JSONValue])
    case array([JSONValue])
    case null

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let bool = try? container.decode(Bool.self) {
            self = .bool(bool)
        } else if let number = try? container.decode(Double.self) {
            self = .number(number)
        } else if let string = try? container.decode(String.self) {
            self = .string(string)
        } else if let array = try? container.decode([JSONValue].self) {
            self = .array(array)
        } else if let object = try? container.decode([String: JSONValue].self) {
            self = .object(object)
        } else {
            throw DecodingError.dataCorruptedError(
                in: container,
                debugDescription: "Unsupported JSON value"
            )
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .string(let value): try container.encode(value)
        case .number(let value): try container.encode(value)
        case .bool(let value): try container.encode(value)
        case .object(let value): try container.encode(value)
        case .array(let value): try container.encode(value)
        case .null: try container.encodeNil()
        }
    }
}

public extension JSONValue {
    /// The value as a `Double` when it is a JSON number, else `nil`. Used by
    /// projections that read numeric metadata (e.g. lat/lng, a score) off an
    /// atom without unwrapping the enum at every call site.
    var doubleValue: Double? {
        if case .number(let value) = self { return value }
        if case .string(let value) = self { return Double(value) }
        return nil
    }

    /// The value as a `String` when it is a JSON string, else `nil`.
    var stringValue: String? {
        if case .string(let value) = self { return value }
        return nil
    }
}
