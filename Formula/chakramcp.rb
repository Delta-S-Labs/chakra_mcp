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
  version "0.1.1"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.1/chakramcp-0.1.1-aarch64-apple-darwin.tar.gz"
      sha256 "a55eb428a7d1e44ee26525f3496520877270991649a20c70f58719dd52475581"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.1/chakramcp-0.1.1-x86_64-apple-darwin.tar.gz"
      sha256 "ee3429fdf39d441080eec36d9e6b69412d1e6b9c16bb5701e83cf7d68797ea64"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.1/chakramcp-0.1.1-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "cc356f41ddab17d56b2f9447c4fb1d82e1f0085bf00db5eaf9853e50441b5f31"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.1/chakramcp-0.1.1-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "4c49f7ce99b871d0fcb06cac11e6baa1147f14b62e5bd4bc522fe6300574400e"
    end
  end

  def install
    bin.install "chakramcp"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp --version")
  end
end
