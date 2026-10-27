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
  version "0.5.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.5.0/chakramcp-0.5.0-aarch64-apple-darwin.tar.gz"
      sha256 "134194159da47b58090ce4b2b8d50c9156829a9d05c6013157946cd53147036c"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.5.0/chakramcp-0.5.0-x86_64-apple-darwin.tar.gz"
      sha256 "8ae75d5a6981406ef3451fa7f94f393e53f39f4fe4b35da1e9c6b8eed11a9f58"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.5.0/chakramcp-0.5.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "01ad6521157d1873e78550b76bd0f16e04ad34d5fafa9c70818e049e904ef407"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.5.0/chakramcp-0.5.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "8431e79484b531929a9e7480fb19152b00113a192ce24b53d9a214d1c4f9ca7d"
    end
  end

  def install
    bin.install "chakramcp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp --version")
  end
end
