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
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.0/chakramcp-0.1.0-aarch64-apple-darwin.tar.gz"
      sha256 "e1272dee88204c8cdc04007df7a07ec66c49448155f3ce8ff7897c3734929c1a"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.0/chakramcp-0.1.0-x86_64-apple-darwin.tar.gz"
      sha256 "94e712cb2aa0f7a81fc0bb92f9611abc724d1a757fac2633e44299a483a3ab2a"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.0/chakramcp-0.1.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "a982d137901b7959c55d78b974312b2f492ad57c024b54f47b858b101258f275"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.0/chakramcp-0.1.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "61b4dc1e1dc7873d709cddc420395544394d75f8dd6785f8739222c33b0b2611"
    end
  end

  def install
    bin.install "chakramcp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp --version")
  end
end
