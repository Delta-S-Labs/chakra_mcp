# Homebrew formula for the chakramcp CLI.
#
# Rendered + committed to Formula/chakramcp.rb by
# .github/workflows/cli-release.yml on every cli-v* release. The
# placeholders below get substituted with the version and per-platform
# sha256s of the tarballs uploaded to the GitHub Release.
#
# To install (the tap is this repo, so it needs the URL):
#   brew tap Delta-S-Labs/chakra_mcp https://github.com/Delta-S-Labs/chakra_mcp
#   brew install Delta-S-Labs/chakra_mcp/chakramcp

class Chakramcp < Formula
  desc "Command-line client for the ChakraMCP relay"
  homepage "https://chakramcp.com"
  version "0.8.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.8.0/chakramcp-0.8.0-aarch64-apple-darwin.tar.gz"
      sha256 "2828a71f9e1c4f4b16ad91209e4b89913951a892c51d4b522229e115010a88a8"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.8.0/chakramcp-0.8.0-x86_64-apple-darwin.tar.gz"
      sha256 "5a25df7b5a038699e6961938dc3b08ee65c8a817922175e78b9c666d333eedf0"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.8.0/chakramcp-0.8.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "7d7f94fec640159db42fbcfe06a1a414cbfb901345144dc66fb9fb82c19c764d"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.8.0/chakramcp-0.8.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "cf1e4c94e0ac84ca24966697305d8f5c0f5ee97efd4b0e7db5978e215ef177e7"
    end
  end

  def install
    bin.install "chakramcp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp --version")
  end
end
