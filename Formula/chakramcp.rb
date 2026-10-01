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
  version "0.3.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.3.0/chakramcp-0.3.0-aarch64-apple-darwin.tar.gz"
      sha256 "93a9f4971620867f01790e7cc1d20a610c382657b04aa0193e89ed3d4af0e0fb"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.3.0/chakramcp-0.3.0-x86_64-apple-darwin.tar.gz"
      sha256 "ae07b6f7d7b269f70f546b980f427f2799cdfbfdd87ca3f3505676f4bdad4e9f"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.3.0/chakramcp-0.3.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "628cc24abb6e6daa0180f6aa7c4c2a817c77b3be282cfbff1f6853b7e91f3256"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.3.0/chakramcp-0.3.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "bc1c8dd7a3701cd4c385ea7b16f10f39088667fda3c42bb22a3201f03e04eb3a"
    end
  end

  def install
    bin.install "chakramcp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp --version")
  end
end
