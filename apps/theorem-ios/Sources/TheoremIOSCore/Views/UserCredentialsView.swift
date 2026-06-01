import SwiftUI

public struct UserCredentialsView: View {
    var theme: TheoremTheme
    @State private var viewModel: UserCredentialsViewModel

    public init(
        theme: TheoremTheme,
        viewModel: UserCredentialsViewModel = UserCredentialsViewModel()
    ) {
        self.theme = theme
        _viewModel = State(initialValue: viewModel)
    }

    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                header
                SearchEndpointSection(viewModel: viewModel, theme: theme)
                ProviderCredentialsSection(viewModel: viewModel, theme: theme)
            }
            .padding(.horizontal, 22)
            .padding(.top, 92)
            .padding(.bottom, 112)
        }
        .scrollContentBackground(.hidden)
        .background(theme.background)
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("Credentials", systemImage: "key")
                .font(TheoremFonts.display(size: 40, relativeTo: .largeTitle))
                .foregroundStyle(theme.textPrimary)
                .labelStyle(.titleAndIcon)
            HStack(spacing: 8) {
                statusPill("Keychain", symbol: "lock")
                statusPill("Endpoints", symbol: "link")
            }
        }
    }

    private func statusPill(_ title: String, symbol: String) -> some View {
        Label(title, systemImage: symbol)
            .font(TheoremFonts.label(size: 11))
            .textCase(.uppercase)
            .foregroundStyle(theme.textSecondary)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(theme.surface, in: Capsule())
    }
}

private struct SearchEndpointSection: View {
    @Bindable var viewModel: UserCredentialsViewModel
    var theme: TheoremTheme

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            SectionHeader(title: "Search", symbol: "magnifyingglass", theme: theme)
            VStack(alignment: .leading, spacing: 12) {
                Picker("Backend", selection: Binding(
                    get: { viewModel.searchBackend },
                    set: { viewModel.setSearchBackend($0) }
                )) {
                    ForEach(TheoremSearchBackend.allCases) { backend in
                        Text(backend.displayName).tag(backend)
                    }
                }
                .pickerStyle(.segmented)

                CredentialTextField(
                    title: "Base URL",
                    text: $viewModel.searchBaseURLText,
                    symbol: "link",
                    theme: theme
                )
                CredentialTextField(
                    title: "Tenant",
                    text: $viewModel.searchTenantText,
                    symbol: "person.crop.square",
                    theme: theme
                )

                HStack(spacing: 10) {
                    CredentialIconButton(
                        symbol: "arrow.uturn.backward",
                        label: "Reset search endpoint",
                        theme: theme,
                        action: viewModel.resetSearchEndpoint
                    )
                    CredentialIconButton(
                        symbol: "checkmark",
                        label: "Save search endpoint",
                        isProminent: true,
                        theme: theme,
                        action: viewModel.saveSearchConfig
                    )
                    Spacer()
                    StatusText(viewModel: viewModel, theme: theme)
                }
            }
            .padding(14)
            .background(theme.surface.opacity(0.92), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .stroke(theme.hairline, lineWidth: 1)
            )
        }
    }
}

private struct ProviderCredentialsSection: View {
    @Bindable var viewModel: UserCredentialsViewModel
    var theme: TheoremTheme

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            SectionHeader(title: "Providers", symbol: "server.rack", theme: theme)
            ForEach(viewModel.providerDrafts.indices, id: \.self) { index in
                ProviderCredentialRow(
                    draft: Binding(
                        get: { viewModel.providerDrafts[index] },
                        set: { viewModel.providerDrafts[index] = $0 }
                    ),
                    theme: theme,
                    onSave: { viewModel.saveProvider(viewModel.providerDrafts[index].id) },
                    onResetEndpoint: { viewModel.resetEndpoint(viewModel.providerDrafts[index].id) },
                    onClearKey: { viewModel.clearAPIKey(viewModel.providerDrafts[index].id) }
                )
            }
        }
    }
}

private struct ProviderCredentialRow: View {
    @Binding var draft: UserCredentialProviderDraft
    var theme: TheoremTheme
    var onSave: () -> Void
    var onResetEndpoint: () -> Void
    var onClearKey: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                VStack(alignment: .leading, spacing: 3) {
                    Text(draft.displayName)
                        .font(TheoremFonts.body(size: 17).weight(.semibold))
                        .foregroundStyle(theme.textPrimary)
                    Text(draft.protocolLabel)
                        .font(TheoremFonts.mono(size: 11))
                        .foregroundStyle(theme.textSecondary)
                }
                Spacer()
                Text(draft.hasStoredAPIKey ? "Key saved" : "No key")
                    .font(TheoremFonts.label(size: 10))
                    .textCase(.uppercase)
                    .foregroundStyle(draft.hasStoredAPIKey ? theme.signal : theme.textSecondary)
            }

            CredentialTextField(
                title: "Endpoint URL",
                text: $draft.endpointText,
                symbol: "link",
                theme: theme
            )

            HStack(spacing: 10) {
                Image(systemName: "lock")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(theme.textSecondary)
                    .frame(width: 18)
                SecureField(draft.hasStoredAPIKey ? "Replace API key" : "API key", text: $draft.apiKeyText)
                    .font(TheoremFonts.mono(size: 13))
                    .textFieldStyle(.plain)
                    .foregroundStyle(theme.textPrimary)
            }
            .padding(.horizontal, 11)
            .frame(height: 42)
            .background(theme.background, in: RoundedRectangle(cornerRadius: 7, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 7, style: .continuous)
                    .stroke(theme.hairline, lineWidth: 1)
            )

            HStack(spacing: 8) {
                CredentialIconButton(
                    symbol: "arrow.uturn.backward",
                    label: "Reset endpoint",
                    theme: theme,
                    action: onResetEndpoint
                )
                CredentialIconButton(
                    symbol: "trash",
                    label: "Clear API key",
                    theme: theme,
                    action: onClearKey
                )
                Spacer()
                CredentialIconButton(
                    symbol: "checkmark",
                    label: "Save provider credentials",
                    isProminent: true,
                    theme: theme,
                    action: onSave
                )
            }
        }
        .padding(14)
        .background(theme.surface.opacity(0.92), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(draft.usesEndpointOverride ? theme.signal.opacity(0.38) : theme.hairline, lineWidth: 1)
        )
    }
}

private struct SectionHeader: View {
    var title: String
    var symbol: String
    var theme: TheoremTheme

    var body: some View {
        Label(title, systemImage: symbol)
            .font(TheoremFonts.label(size: 12))
            .textCase(.uppercase)
            .foregroundStyle(theme.textSecondary)
    }
}

private struct CredentialTextField: View {
    var title: String
    @Binding var text: String
    var symbol: String
    var theme: TheoremTheme

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: symbol)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(theme.textSecondary)
                .frame(width: 18)
            TextField(title, text: $text)
                .font(TheoremFonts.mono(size: 13))
                .textFieldStyle(.plain)
                .foregroundStyle(theme.textPrimary)
        }
        .padding(.horizontal, 11)
        .frame(height: 42)
        .background(theme.background, in: RoundedRectangle(cornerRadius: 7, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 7, style: .continuous)
                .stroke(theme.hairline, lineWidth: 1)
        )
    }
}

private struct CredentialIconButton: View {
    var symbol: String
    var label: String
    var isProminent = false
    var theme: TheoremTheme
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 14, weight: .bold))
                .frame(width: 38, height: 34)
        }
        .buttonStyle(.plain)
        .foregroundStyle(isProminent ? theme.surface : theme.textPrimary)
        .background(isProminent ? theme.textPrimary : theme.background, in: RoundedRectangle(cornerRadius: 7, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 7, style: .continuous)
                .stroke(isProminent ? Color.clear : theme.hairline, lineWidth: 1)
        )
        .help(label)
        .accessibilityLabel(label)
    }
}

private struct StatusText: View {
    var viewModel: UserCredentialsViewModel
    var theme: TheoremTheme

    var body: some View {
        Text(viewModel.errorMessage ?? viewModel.statusMessage ?? "")
            .font(TheoremFonts.mono(size: 11))
            .foregroundStyle(viewModel.errorMessage == nil ? theme.textSecondary : theme.signal)
            .lineLimit(2)
            .multilineTextAlignment(.trailing)
    }
}
