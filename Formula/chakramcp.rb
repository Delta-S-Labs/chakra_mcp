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
  version "0.1.3"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.3/chakramcp-0.1.3-aarch64-apple-darwin.tar.gz"
      sha256 "56505a6f4968aecfff9f9393dee2a83b4d6268f8a2109ee35832565bf2e19c06"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.3/chakramcp-0.1.3-x86_64-apple-darwin.tar.gz"
      sha256 "7ea6d861add517e3cab3db7f0925b126b76f7750916f783c25a246df4bd57196"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.3/chakramcp-0.1.3-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "1a90503718939b2639a60a26b34dae950ed30469952641220e41f7557dc681fa"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.3/chakramcp-0.1.3-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "a63b624233e916963397d640b1457de131bfa30843a74426068af4d591fb1ca1"
    end
  end

  def install
    bin.install "chakramcp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp --version")
  end
end
