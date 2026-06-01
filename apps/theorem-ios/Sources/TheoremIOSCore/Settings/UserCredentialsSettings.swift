import Foundation
import Observation
import Security

public enum UserCredentialSettingsError: Error, Equatable {
    case invalidEndpoint(String)
    case keychainStatus(Int32)
    case keychainDecodeFailed

    public var message: String {
        switch self {
        case .invalidEndpoint:
            "Endpoint must be an http or https URL."
        case .keychainStatus:
            "Keychain could not save this API key."
        case .keychainDecodeFailed:
            "Keychain returned an unreadable API key."
        }
    }
}

@MainActor
public protocol TheoremAPIKeyStoring: AnyObject {
    func readAPIKey(providerID: String) throws -> String?
    func saveAPIKey(_ apiKey: String, providerID: String) throws
    func deleteAPIKey(providerID: String) throws
}

@MainActor
public protocol UserCredentialConfigStoring: AnyObject {
    func endpointOverride(providerID: String) -> String?
    func setEndpointOverride(_ endpoint: String?, providerID: String)
    func searchBackend() -> TheoremSearchBackend
    func setSearchBackend(_ backend: TheoremSearchBackend)
    func searchBaseURLOverride() -> String?
    func setSearchBaseURLOverride(_ endpoint: String?)
    func searchTenant() -> String?
    func setSearchTenant(_ tenant: String?)
}

public final class KeychainTheoremAPIKeyStore: TheoremAPIKeyStoring {
    private let service: String

    public init(service: String = "com.travisgilbert.theorem-ios.api-keys") {
        self.service = service
    }

    public func readAPIKey(providerID: String) throws -> String? {
        var query = baseQuery(providerID: providerID)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne

        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else {
            throw UserCredentialSettingsError.keychainStatus(status)
        }
        guard let data = result as? Data,
              let apiKey = String(data: data, encoding: .utf8) else {
            throw UserCredentialSettingsError.keychainDecodeFailed
        }
        return apiKey
    }

    public func saveAPIKey(_ apiKey: String, providerID: String) throws {
        let trimmed = apiKey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            try deleteAPIKey(providerID: providerID)
            return
        }

        let data = Data(trimmed.utf8)
        let query = baseQuery(providerID: providerID)
        let update = [kSecValueData as String: data]
        let updateStatus = SecItemUpdate(query as CFDictionary, update as CFDictionary)
        if updateStatus == errSecSuccess { return }
        if updateStatus != errSecItemNotFound {
            throw UserCredentialSettingsError.keychainStatus(updateStatus)
        }

        var add = query
        add[kSecValueData as String] = data
        let addStatus = SecItemAdd(add as CFDictionary, nil)
        guard addStatus == errSecSuccess else {
            throw UserCredentialSettingsError.keychainStatus(addStatus)
        }
    }

    public func deleteAPIKey(providerID: String) throws {
        let status = SecItemDelete(baseQuery(providerID: providerID) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw UserCredentialSettingsError.keychainStatus(status)
        }
    }

    private func baseQuery(providerID: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: providerID,
        ]
    }
}

public final class UserDefaultsCredentialConfigStore: UserCredentialConfigStoring {
    public static let searchBackendKey = "searchBackend"
    public static let searchBaseURLKey = "searchBaseURL"
    public static let searchTenantKey = "searchTenant"

    private let defaults: UserDefaults

    public init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    public func endpointOverride(providerID: String) -> String? {
        trimmed(defaults.string(forKey: Self.providerEndpointKey(providerID)))
    }

    public func setEndpointOverride(_ endpoint: String?, providerID: String) {
        setOptional(endpoint, forKey: Self.providerEndpointKey(providerID))
    }

    public func searchBackend() -> TheoremSearchBackend {
        let raw = defaults.string(forKey: Self.searchBackendKey)
            ?? defaults.string(forKey: "theoremSearchBackend")
        return raw.flatMap(TheoremSearchBackend.init(rawValue:)) ?? .rustyRed
    }

    public func setSearchBackend(_ backend: TheoremSearchBackend) {
        defaults.set(backend.rawValue, forKey: Self.searchBackendKey)
    }

    public func searchBaseURLOverride() -> String? {
        trimmed(defaults.string(forKey: Self.searchBaseURLKey)
            ?? defaults.string(forKey: "theoremSearchBaseURL"))
    }

    public func setSearchBaseURLOverride(_ endpoint: String?) {
        setOptional(endpoint, forKey: Self.searchBaseURLKey)
    }

    public func searchTenant() -> String? {
        trimmed(defaults.string(forKey: Self.searchTenantKey)
            ?? defaults.string(forKey: "theoremSearchTenant"))
    }

    public func setSearchTenant(_ tenant: String?) {
        setOptional(tenant, forKey: Self.searchTenantKey)
    }

    private static func providerEndpointKey(_ providerID: String) -> String {
        "userCredentials.endpoint.\(providerID)"
    }

    private func setOptional(_ value: String?, forKey key: String) {
        if let value = trimmed(value) {
            defaults.set(value, forKey: key)
        } else {
            defaults.removeObject(forKey: key)
        }
    }

    private func trimmed(_ value: String?) -> String? {
        guard let value = value?.trimmingCharacters(in: .whitespacesAndNewlines),
              !value.isEmpty else { return nil }
        return value
    }
}

public struct UserCredentialProviderDraft: Equatable, Identifiable {
    public var id: String
    public var displayName: String
    public var protocolLabel: String
    public var defaultEndpointText: String
    public var endpointText: String
    public var apiKeyText: String
    public var hasStoredAPIKey: Bool
    public var lastSavedEndpointText: String

    public var usesEndpointOverride: Bool {
        endpointText.trimmingCharacters(in: .whitespacesAndNewlines) != defaultEndpointText
    }
}

@MainActor
@Observable
public final class UserCredentialsViewModel {
    public var providerDrafts: [UserCredentialProviderDraft]
    public var searchBackend: TheoremSearchBackend
    public var searchBaseURLText: String
    public var searchTenantText: String
    public var statusMessage: String?
    public var errorMessage: String?

    private let configStore: any UserCredentialConfigStoring
    private let apiKeyStore: any TheoremAPIKeyStoring

    public init(
        registry: CommonplaceRegistry = SampleCommonplaceRegistry.registry,
        configStore: any UserCredentialConfigStoring = UserDefaultsCredentialConfigStore(),
        apiKeyStore: any TheoremAPIKeyStoring = KeychainTheoremAPIKeyStore()
    ) {
        self.configStore = configStore
        self.apiKeyStore = apiKeyStore

        let backend = configStore.searchBackend()
        self.searchBackend = backend
        self.searchBaseURLText = configStore.searchBaseURLOverride()
            ?? Self.defaultSearchBaseURL(for: backend).absoluteString
        self.searchTenantText = configStore.searchTenant() ?? ""
        self.providerDrafts = Self.providerDrafts(
            registry: registry,
            configStore: configStore,
            apiKeyStore: apiKeyStore
        )
    }

    public func setSearchBackend(_ backend: TheoremSearchBackend) {
        let priorDefault = Self.defaultSearchBaseURL(for: searchBackend).absoluteString
        let shouldFollowDefault = searchBaseURLText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || searchBaseURLText == priorDefault
        searchBackend = backend
        if shouldFollowDefault {
            searchBaseURLText = Self.defaultSearchBaseURL(for: backend).absoluteString
        }
    }

    public func saveSearchConfig() {
        do {
            let endpoint = try Self.normalizedEndpoint(searchBaseURLText)
            configStore.setSearchBackend(searchBackend)
            configStore.setSearchBaseURLOverride(endpoint)
            configStore.setSearchTenant(searchTenantText)
            statusMessage = "Search endpoint saved."
            errorMessage = nil
        } catch let error as UserCredentialSettingsError {
            errorMessage = error.message
        } catch {
            errorMessage = "Search endpoint could not be saved."
        }
    }

    public func resetSearchEndpoint() {
        configStore.setSearchBaseURLOverride(nil)
        searchBaseURLText = Self.defaultSearchBaseURL(for: searchBackend).absoluteString
        statusMessage = "Search endpoint reset."
        errorMessage = nil
    }

    public func saveProvider(_ providerID: String) {
        guard let index = providerDrafts.firstIndex(where: { $0.id == providerID }) else { return }
        do {
            let endpoint = try Self.normalizedEndpoint(providerDrafts[index].endpointText)
            let defaultEndpoint = providerDrafts[index].defaultEndpointText
            configStore.setEndpointOverride(endpoint == defaultEndpoint ? nil : endpoint, providerID: providerID)

            let apiKey = providerDrafts[index].apiKeyText.trimmingCharacters(in: .whitespacesAndNewlines)
            if !apiKey.isEmpty {
                try apiKeyStore.saveAPIKey(apiKey, providerID: providerID)
                providerDrafts[index].apiKeyText = ""
                providerDrafts[index].hasStoredAPIKey = true
            }

            providerDrafts[index].lastSavedEndpointText = providerDrafts[index].endpointText
            statusMessage = "\(providerDrafts[index].displayName) saved."
            errorMessage = nil
        } catch let error as UserCredentialSettingsError {
            errorMessage = error.message
        } catch {
            errorMessage = "\(providerDrafts[index].displayName) could not be saved."
        }
    }

    public func resetEndpoint(_ providerID: String) {
        guard let index = providerDrafts.firstIndex(where: { $0.id == providerID }) else { return }
        configStore.setEndpointOverride(nil, providerID: providerID)
        providerDrafts[index].endpointText = providerDrafts[index].defaultEndpointText
        providerDrafts[index].lastSavedEndpointText = providerDrafts[index].defaultEndpointText
        statusMessage = "\(providerDrafts[index].displayName) endpoint reset."
        errorMessage = nil
    }

    public func clearAPIKey(_ providerID: String) {
        guard let index = providerDrafts.firstIndex(where: { $0.id == providerID }) else { return }
        do {
            try apiKeyStore.deleteAPIKey(providerID: providerID)
            providerDrafts[index].apiKeyText = ""
            providerDrafts[index].hasStoredAPIKey = false
            statusMessage = "\(providerDrafts[index].displayName) key cleared."
            errorMessage = nil
        } catch {
            errorMessage = "\(providerDrafts[index].displayName) key could not be cleared."
        }
    }

    public static func normalizedEndpoint(_ raw: String) throws -> String? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        guard let components = URLComponents(string: trimmed),
              let scheme = components.scheme?.lowercased(),
              ["http", "https"].contains(scheme),
              components.host?.isEmpty == false else {
            throw UserCredentialSettingsError.invalidEndpoint(raw)
        }
        return trimmed.removingTrailingSlashes()
    }

    private static func providerDrafts(
        registry: CommonplaceRegistry,
        configStore: any UserCredentialConfigStoring,
        apiKeyStore: any TheoremAPIKeyStoring
    ) -> [UserCredentialProviderDraft] {
        registry.providers
            .filter { provider in
                provider.baseURL != nil
                    && provider.protocolKind != .localRuntime
                    && provider.credentialMode != .localOnly
            }
            .map { provider in
                let defaultEndpoint = provider.baseURL?.absoluteString.removingTrailingSlashes() ?? ""
                let endpoint = configStore.endpointOverride(providerID: provider.id) ?? defaultEndpoint
                let hasKey = (try? apiKeyStore.readAPIKey(providerID: provider.id))?.isEmpty == false
                return UserCredentialProviderDraft(
                    id: provider.id,
                    displayName: provider.displayName,
                    protocolLabel: provider.protocolKind.displayName,
                    defaultEndpointText: defaultEndpoint,
                    endpointText: endpoint,
                    apiKeyText: "",
                    hasStoredAPIKey: hasKey,
                    lastSavedEndpointText: endpoint
                )
            }
    }

    private static func defaultSearchBaseURL(for backend: TheoremSearchBackend) -> URL {
        switch backend {
        case .indexAPI:
            TheoremSearchClient.productionIndexAPIBaseURL
        case .rustyRed:
            TheoremSearchClient.productionRustyRedBaseURL
        }
    }
}

extension TheoremSearchBackend: CaseIterable, Identifiable {
    // CaseIterable cannot be auto-synthesized in an extension outside the enum's
    // declaring file (TheoremSearchClient.swift), so provide allCases explicitly.
    public static var allCases: [TheoremSearchBackend] { [.indexAPI, .rustyRed] }

    public var id: String { rawValue }

    public var displayName: String {
        switch self {
        case .indexAPI:
            "Index API"
        case .rustyRed:
            "RustyRed"
        }
    }
}

private extension CommonplaceProviderProtocol {
    var displayName: String {
        switch self {
        case .openAIChat:
            "OpenAI-compatible"
        case .openAIResponses:
            "Responses"
        case .anthropicMessages:
            "Anthropic"
        case .harness:
            "Harness"
        case .localRuntime:
            "Local"
        }
    }
}

private extension String {
    func removingTrailingSlashes() -> String {
        var output = self
        while output.count > 1, output.hasSuffix("/") {
            output.removeLast()
        }
        return output
    }
}
