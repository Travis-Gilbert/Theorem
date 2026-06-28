# Homebrew formula for theorem-proxy (roadmap C.1 brew tap).
# Installs the prebuilt binary attached to a `proxy-v*` GitHub release by
# .github/workflows/release-proxy.yml. Fill VERSION + the three sha256 values from the
# release's .sha256 assets (or let a tap-update step rewrite them on release).
class TheoremProxy < Formula
  desc "Local Anthropic Messages proxy that makes the Theorem harness ambient"
  homepage "https://github.com/Travis-Gilbert/Theorem"
  version "0.1.0"

  on_macos do
    on_arm do
      url "https://github.com/Travis-Gilbert/Theorem/releases/download/proxy-v0.1.0/theorem-proxy-aarch64-apple-darwin"
      sha256 "REPLACE_WITH_aarch64-apple-darwin_SHA256"
    end
    on_intel do
      url "https://github.com/Travis-Gilbert/Theorem/releases/download/proxy-v0.1.0/theorem-proxy-x86_64-apple-darwin"
      sha256 "REPLACE_WITH_x86_64-apple-darwin_SHA256"
    end
  end

  on_linux do
    url "https://github.com/Travis-Gilbert/Theorem/releases/download/proxy-v0.1.0/theorem-proxy-x86_64-unknown-linux-gnu"
    sha256 "REPLACE_WITH_x86_64-unknown-linux-gnu_SHA256"
  end

  def install
    # The release asset is the bare binary named per-target; install it as `theorem-proxy`.
    bin.install Dir["theorem-proxy-*"].first => "theorem-proxy"
  end

  test do
    assert_match "theorem-proxy", shell_output("#{bin}/theorem-proxy --help")
  end
end
