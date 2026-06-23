import Foundation
import XCTest
@testable import TheoremIOSCore

@MainActor
final class UserCredentialsSettingsTests: XCTestCase {
    func testSavesProviderEndpointToDefaultsAndAPIKeyToSecretStore() {
        let defaults = isolatedDefaults()
        defer { clear(defaults) }
        let configStore = UserDefaultsCredentialConfigStore(defaults: defaults)
        let apiKeyStore = MemoryAPIKeyStore()
        let viewModel = UserCredentialsViewModel(
            registry: Self.registry(),
            configStore: configStore,
            apiKeyStore: apiKeyStore
        )

        XCTAssertEqual(viewModel.providerDrafts.map(\.id), ["mistral"])
        viewModel.providerDrafts[0].endpointText = "https://bring.example.com/v1/"
        viewModel.providerDrafts[0].apiKeyText = "  sk-test  "
        viewModel.saveProvider("mistral")

        XCTAssertEqual(configStore.endpointOverride(providerID: "mistral"), "https://bring.example.com/v1")
        XCTAssertEqual(apiKeyStore.keys["mistral"], "sk-test")
        XCTAssertTrue(viewModel.providerDrafts[0].hasStoredAPIKey)
        XCTAssertEqual(viewModel.providerDrafts[0].apiKeyText, "")
        XCTAssertFalse(defaults.dictionaryRepresentation().values.contains { value in
            (value as? String)?.contains("sk-test") == true
        })
    }

    func testResetEndpointRemovesProviderOverride() {
        let defaults = isolatedDefaults()
        defer { clear(defaults) }
        let configStore = UserDefaultsCredentialConfigStore(defaults: defaults)
        let viewModel = UserCredentialsViewModel(
            registry: Self.registry(),
            configStore: configStore,
            apiKeyStore: MemoryAPIKeyStore()
        )

        viewModel.providerDrafts[0].endpointText = "https://bring.example.com"
        viewModel.saveProvider("mistral")
        XCTAssertEqual(configStore.endpointOverride(providerID: "mistral"), "https://bring.example.com")

        viewModel.resetEndpoint("mistral")

        XCTAssertNil(configStore.endpointOverride(providerID: "mistral"))
        XCTAssertEqual(viewModel.providerDrafts[0].endpointText, "https://api.mistral.ai")
    }

    func testInvalidProviderEndpointDoesNotPersist() {
        let defaults = isolatedDefaults()
        defer { clear(defaults) }
        let configStore = UserDefaultsCredentialConfigStore(defaults: defaults)
        let apiKeyStore = MemoryAPIKeyStore()
        let viewModel = UserCredentialsViewModel(
            registry: Self.registry(),
            configStore: configStore,
            apiKeyStore: apiKeyStore
        )

        viewModel.providerDrafts[0].endpointText = "ftp://example.com"
        viewModel.providerDrafts[0].apiKeyText = "sk-test"
        viewModel.saveProvider("mistral")

        XCTAssertEqual(viewModel.errorMessage, "Endpoint must be an http or https URL.")
        XCTAssertNil(configStore.endpointOverride(providerID: "mistral"))
        XCTAssertNil(apiKeyStore.keys["mistral"])
    }

    func testSearchConfigWritesExistingSearchClientKeys() {
        let defaults = isolatedDefaults()
        defer { clear(defaults) }
        let configStore = UserDefaultsCredentialConfigStore(defaults: defaults)
        let viewModel = UserCredentialsViewModel(
            registry: Self.registry(),
            configStore: configStore,
            apiKeyStore: MemoryAPIKeyStore()
        )

        viewModel.setSearchBackend(.indexAPI)
        viewModel.searchBaseURLText = "https://index.example.com/theorem/"
        viewModel.searchTenantText = "demo"
        viewModel.saveSearchConfig()

        XCTAssertEqual(defaults.string(forKey: UserDefaultsCredentialConfigStore.searchBackendKey), "index-api")
        XCTAssertEqual(defaults.string(forKey: UserDefaultsCredentialConfigStore.searchBaseURLKey), "https://index.example.com/theorem")
        XCTAssertEqual(defaults.string(forKey: UserDefaultsCredentialConfigStore.searchTenantKey), "demo")
    }

    func testClearAPIKeyDeletesSecretWithoutTouchingEndpoint() {
        let defaults = isolatedDefaults()
        defer { clear(defaults) }
        let configStore = UserDefaultsCredentialConfigStore(defaults: defaults)
        let apiKeyStore = MemoryAPIKeyStore()
        let viewModel = UserCredentialsViewModel(
            registry: Self.registry(),
            configStore: configStore,
            apiKeyStore: apiKeyStore
        )

        viewModel.providerDrafts[0].endpointText = "https://bring.example.com"
        viewModel.providerDrafts[0].apiKeyText = "sk-test"
        viewModel.saveProvider("mistral")
        viewModel.clearAPIKey("mistral")

        XCTAssertNil(apiKeyStore.keys["mistral"])
        XCTAssertFalse(viewModel.providerDrafts[0].hasStoredAPIKey)
        XCTAssertEqual(configStore.endpointOverride(providerID: "mistral"), "https://bring.example.com")
    }

    private static func registry() -> CommonplaceRegistry {
        CommonplaceRegistry(
            id: "test-registry",
            providers: [
                CommonplaceProvider(
                    id: "mistral",
                    displayName: "Mistral",
                    baseURL: URL(string: "https://api.mistral.ai"),
                    protocolKind: .openAIChat,
                    credentialMode: .platformManaged
                ),
                CommonplaceProvider(
                    id: "on-device",
                    displayName: "On-device",
                    protocolKind: .localRuntime,
                    credentialMode: .localOnly,
                    probePath: nil
                ),
            ],
            participantBindings: [],
            machineryBindings: [],
            charter: CommonplaceTeamCharter(
                id: "test-charter",
                title: "Test",
                promptSeed: "",
                principles: [],
                substrateInstructions: []
            )
        )
    }

    private func isolatedDefaults() -> UserDefaults {
        let suiteName = "TheoremIOSCoreTests.UserCredentials.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defaults.set(suiteName, forKey: "_suiteName")
        return defaults
    }

    private func clear(_ defaults: UserDefaults) {
        guard let suiteName = defaults.string(forKey: "_suiteName") else { return }
        defaults.removePersistentDomain(forName: suiteName)
    }
}

@MainActor
private final class MemoryAPIKeyStore: TheoremAPIKeyStoring {
    var keys: [String: String] = [:]

    func readAPIKey(providerID: String) throws -> String? {
        keys[providerID]
    }

    func saveAPIKey(_ apiKey: String, providerID: String) throws {
        let trimmed = apiKey.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            keys.removeValue(forKey: providerID)
        } else {
            keys[providerID] = trimmed
        }
    }

    func deleteAPIKey(providerID: String) throws {
        keys.removeValue(forKey: providerID)
    }
}
