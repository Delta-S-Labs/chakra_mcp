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
  version "0.2.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.2.0/chakramcp-0.2.0-aarch64-apple-darwin.tar.gz"
      sha256 "06f67cb9005dfa3410fdf9312dd9565248daf8654afccb532dcbced6280f4d59"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.2.0/chakramcp-0.2.0-x86_64-apple-darwin.tar.gz"
      sha256 "549aceefc4408743721f0277f847a320ec21907041374857dafd204e2180c2a1"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.2.0/chakramcp-0.2.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "59a7b2cea9ff49d3a465fdefdc3fc45c1a4291b05eb7609c329421d2e2aae476"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.2.0/chakramcp-0.2.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "9b3ac0edfb57047483cf31832ef2c64e6447ad437acb97381055a1808da2de9b"
    end
  end

  def install
    bin.install "chakramcp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp --version")
  end
end
