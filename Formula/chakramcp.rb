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
  version "0.1.2"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.2/chakramcp-0.1.2-aarch64-apple-darwin.tar.gz"
      sha256 "a0a1d7837e517660de81152d8ca6bd44d700c4547efec287677e008dc7a61711"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.2/chakramcp-0.1.2-x86_64-apple-darwin.tar.gz"
      sha256 "c458aa14a3b03dbcaec0dba110bee39569ac6b90c9ec818d9c0e1f8d5db45a66"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.2/chakramcp-0.1.2-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "8daa3cb1cc5ece631d5dbd6a9d386706a4ed1a0dc3e88aa9fd2a96f0a367ccb1"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.2/chakramcp-0.1.2-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "95c2fe817664823f85db4350d632ae21d8fd53538a670d3156c56f8c1fd9fedc"
    end
  end

  def install
    bin.install "chakramcp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp --version")
  end
end
