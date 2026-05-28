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
  version "0.1.4"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.4/chakramcp-0.1.4-aarch64-apple-darwin.tar.gz"
      sha256 "eaa4fba6c0730f3d9010cb1299282ae43d32d6cbf0780956f4e2ef0c889c9b15"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.4/chakramcp-0.1.4-x86_64-apple-darwin.tar.gz"
      sha256 "128ffb7bc0a956baf57c1210c841148a518912b5d971abb76721e2ffa7d32993"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.4/chakramcp-0.1.4-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "dd9e6d6f9236c3b06854b47fabd7c39ba2e527d1540d6357e5434d227ecc9a31"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.4/chakramcp-0.1.4-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "785b99c1ce06c73ddd5190a94196c9205dab1cf87f647dbe43115c180f56cb90"
    end
  end

  def install
    bin.install "chakramcp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp --version")
  end
end
