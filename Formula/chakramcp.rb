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
  version "0.6.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.6.0/chakramcp-0.6.0-aarch64-apple-darwin.tar.gz"
      sha256 "84972bd8a33b3fb9152c29bd26d3c279a0bca7c3f2fa6a7335f9fd8119124653"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.6.0/chakramcp-0.6.0-x86_64-apple-darwin.tar.gz"
      sha256 "adc65a4c2b24dcac36e0d311417a038af7872018b372bb42a07ae6c25c789056"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.6.0/chakramcp-0.6.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "2e2cfdd75b8e2453f9ce42490fa18988e8581fb142dc8a41dcf6978d5d9c0ccd"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.6.0/chakramcp-0.6.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "32ff0d58c61526dc5d1f0d32bdeb69165b2205b7ea87d8a47c95208ceb6f4739"
    end
  end

  def install
    bin.install "chakramcp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp --version")
  end
end
