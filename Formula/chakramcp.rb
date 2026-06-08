# Homebrew formula for the chakramcp CLI.
#
# Rendered + committed to the tap repo by .github/workflows/cli-release.yml
# on every cli-v* release. The placeholders below get substituted with
# the version and per-platform sha256s of the tarballs uploaded to the
# GitHub Release.
#
# To install once the tap is published:
#   brew tap delta-s-labs/chakramcp
#   brew install chakramcp

class Chakramcp < Formula
  desc "Command-line client for the ChakraMCP relay"
  homepage "https://chakramcp.com"
  version "0.1.7"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.7/chakramcp-0.1.7-aarch64-apple-darwin.tar.gz"
      sha256 "7f02503df24965f420a3bdf759022f0df6b0a37477bec5f17c5d0ea2033b8534"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.7/chakramcp-0.1.7-x86_64-apple-darwin.tar.gz"
      sha256 "c10caf42a648593e20c74d6a08469da633c0e217ab4d459f682fb821337d4767"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.7/chakramcp-0.1.7-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "ca755cc26009522c9b5f6b58b216f2a06562616fa5b5c94c4431ec4218c0c995"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.7/chakramcp-0.1.7-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "5f96d9c0143d341fde18d991135a09b0bae06a72051eb7bc36a33cce2ee086f6"
    end
  end

  def install
    bin.install "chakramcp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp --version")
  end
end
