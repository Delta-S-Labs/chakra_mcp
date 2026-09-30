# Homebrew formula for chakramcp-server — runs a private ChakraMCP
# network on the user's machine. Pairs with postgresql@16 (installed
# automatically as a dependency); each is started independently with
# `brew services`.
#
# Rendered + committed to Formula/chakramcp-server.rb on every cli-v*
# release by .github/workflows/cli-release.yml.

class ChakramcpServer < Formula
  desc "Self-hosted ChakraMCP relay (app + relay services in one process)"
  homepage "https://chakramcp.com"
  version "0.2.0"
  license "MIT"

  depends_on "postgresql@16"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.2.0/chakramcp-server-0.2.0-aarch64-apple-darwin.tar.gz"
      sha256 "10fd64aea92b63bb259f9f89bc61a40a2219d43652bf1a7d1e78298eaaadc404"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.2.0/chakramcp-server-0.2.0-x86_64-apple-darwin.tar.gz"
      sha256 "7fd63fdf65918775936a43d636707beb5e612c1f2a3a3858aa2707e5727ff116"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.2.0/chakramcp-server-0.2.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "bcde1dcdff850c18560d44cb220756851a70764f1f30cfe9b07d61da81eaa877"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.2.0/chakramcp-server-0.2.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "756ba239d37e52cc731a859dbe0c0608a4d02dc0caf2c98666c8098a1b3f3223"
    end
  end

  def install
    bin.install "chakramcp-server"
  end

  service do
    run [opt_bin/"chakramcp-server", "start"]
    keep_alive true
    log_path   var/"log/chakramcp-server.log"
    error_log_path var/"log/chakramcp-server.log"
  end

  def caveats
    <<~EOS
      First-time bootstrap (one-time):

        brew services start postgresql@16
        createdb chakramcp
        chakramcp-server init                    # writes ~/.chakramcp/server.toml
        chakramcp-server migrate                 # applies SQL migrations

      Then start it:

        brew services start chakramcp-server     # backgrounds the supervisor
        # — or run in the foreground for logs:
        chakramcp-server start

      The app service answers on http://localhost:8080 and the relay
      on http://localhost:8090. Point the CLI at it with:

        chakramcp networks add private \
          --app-url http://localhost:8080 \
          --relay-url http://localhost:8090
        chakramcp login --network private

      Edit ~/.chakramcp/server.toml to change ports, the JWT secret,
      or admin_email. The web UI is optional and isn't bundled — clone
      https://github.com/Delta-S-Labs/chakra_mcp and run pnpm dev under frontend/
      if you want it.
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp-server --version")
  end
end
