#if canImport(UIKit) && canImport(SafariServices) && os(iOS)
import SafariServices
import SwiftUI

public struct SafariReaderView: UIViewControllerRepresentable {
    public var url: URL

    public init(url: URL) {
        self.url = url
    }

    public func makeUIViewController(context: Context) -> SFSafariViewController {
        SFSafariViewController(url: url)
    }

    public func updateUIViewController(_ controller: SFSafariViewController, context: Context) {}
}
#endif
