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
  version "0.7.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.7.0/chakramcp-0.7.0-aarch64-apple-darwin.tar.gz"
      sha256 "175b2c0e5e8c0960c936a91b5109e4a3ae86dfae948377b74bb82c66c45e501a"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.7.0/chakramcp-0.7.0-x86_64-apple-darwin.tar.gz"
      sha256 "b01c710864846a23ee64b53f0cec40936ecae1d163319fc4d02313e6a743edf2"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.7.0/chakramcp-0.7.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "1c7c3fb70ba2dc1b0f406faddc9d4465e70b2ad82b14f3d8b170dddc4b7f6dc7"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.7.0/chakramcp-0.7.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "c2b0c9e6e16d9ab744139ad0c569a553e1a26b8dca03ddd9055819290f61c90c"
    end
  end

  def install
    bin.install "chakramcp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp --version")
  end
end
