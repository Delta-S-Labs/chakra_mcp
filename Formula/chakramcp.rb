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
  version "0.1.9"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.9/chakramcp-0.1.9-aarch64-apple-darwin.tar.gz"
      sha256 "1711c0762e8b01a5f697988492645d825312a0562d1fa67d82055aec029e4f04"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.9/chakramcp-0.1.9-x86_64-apple-darwin.tar.gz"
      sha256 "0bfbf6c9e58317908a1d36daa53db5eff5010276d2d711b3bd745414b8fb553f"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.9/chakramcp-0.1.9-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "60c7dcdf45d0eb5f5eb80afe6b7670bcdd0c24972f756bfe2418a21d020d7076"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.9/chakramcp-0.1.9-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "1298334f30bed8b289a9d608d5fed669e5688f25185a33981f2120ec2adcadec"
    end
  end

  def install
    bin.install "chakramcp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp --version")
  end
end
