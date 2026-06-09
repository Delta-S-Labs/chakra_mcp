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
  version "0.1.8"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.8/chakramcp-0.1.8-aarch64-apple-darwin.tar.gz"
      sha256 "31180427d692b6474c139f342591318b0f67933f870fbe62b788ed0c57a0befb"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.8/chakramcp-0.1.8-x86_64-apple-darwin.tar.gz"
      sha256 "dc2d30d5a4bc27816915146430c96e4f03e8336306a415a8e892e9dc07fcd037"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.8/chakramcp-0.1.8-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "6944b5cf44c5bb15bffb87a3e1bbcb29fe9260a490eb91d0f4c9b4108e292ec6"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.8/chakramcp-0.1.8-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "ca7353277bd9fefd9601292a2acd7f5ce6347e6d9eafd4fd8d7a790176640f63"
    end
  end

  def install
    bin.install "chakramcp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp --version")
  end
end
