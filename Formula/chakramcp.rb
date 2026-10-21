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
  version "0.4.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.4.0/chakramcp-0.4.0-aarch64-apple-darwin.tar.gz"
      sha256 "0ce689373731802c94c9468411f053f89e3d3dcf6d2ebb7bdf9d3961af4b428d"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.4.0/chakramcp-0.4.0-x86_64-apple-darwin.tar.gz"
      sha256 "f680b5ee535f906d539df2b79a4eca0d3ce9e76edc2f00557387d227c6d7cb05"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.4.0/chakramcp-0.4.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "fb6ce0440c7a1c35765ec4661ea679fef419e860567f1a3f79a69de841f998bc"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.4.0/chakramcp-0.4.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "60996a542e8ecc33886e964f624b47d37c80867e8f477253cd22a396fd4f9b46"
    end
  end

  def install
    bin.install "chakramcp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp --version")
  end
end
