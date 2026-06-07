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
  version "0.1.6"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.6/chakramcp-0.1.6-aarch64-apple-darwin.tar.gz"
      sha256 "1cd71960654048260adcebfe9aad48dfcb9a3d64cb17aff233b6ba745809131b"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.6/chakramcp-0.1.6-x86_64-apple-darwin.tar.gz"
      sha256 "9994da91ca6dc4c6a60fe8eebe2e4c20ff5d5cbe49f50c5faf5f2f02dd9f0bf1"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.6/chakramcp-0.1.6-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "0993f1f17528324cdc25479bdfec46b6764c9b76d903f1feb91ace11ab1c7693"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.6/chakramcp-0.1.6-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "66c8c104c9510fa3297bcba45d1eda05b22aef29ae09267297b006940376f59d"
    end
  end

  def install
    bin.install "chakramcp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp --version")
  end
end
