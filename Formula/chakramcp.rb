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
  version "0.1.5"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.5/chakramcp-0.1.5-aarch64-apple-darwin.tar.gz"
      sha256 "a32ff14a3d0ec06282cc0c4c0b74f66dcf9fdefb452477a009f8b708d03435d5"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.5/chakramcp-0.1.5-x86_64-apple-darwin.tar.gz"
      sha256 "ae65eb57ad9de39d163d582a4d3f49ef281336a75a87cf4734660104fdc63a95"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.5/chakramcp-0.1.5-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "814dd8c9422523f608f583ac748a88b5497030fd75a54bde1e03943986a9981a"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.5/chakramcp-0.1.5-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "531979f03e7a614f82ced8ca1213fc17c398ceeb8fdc8e5d64d722d92b6bcc7e"
    end
  end

  def install
    bin.install "chakramcp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp --version")
  end
end
