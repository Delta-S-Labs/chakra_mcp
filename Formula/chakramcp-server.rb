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
  version "0.1.6"
  license "MIT"

  depends_on "postgresql@16"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.6/chakramcp-server-0.1.6-aarch64-apple-darwin.tar.gz"
      sha256 "4c9498d31547ec60d999250fe450267c7d5bbb744eaa728fde1ae24c666c66ba"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.6/chakramcp-server-0.1.6-x86_64-apple-darwin.tar.gz"
      sha256 "7e224f0c37b160b5e5e97ef35398fd38701dec57d75d7ecca7dfa4c93a744812"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.6/chakramcp-server-0.1.6-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "f1dbadbdd63709577dc1f23001cc2a007498aeb0577c1729ce6607f40b66a58c"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.6/chakramcp-server-0.1.6-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "9e034f0ea7351dede54258191c3331ea7c073a541456220eeeb4525cf900741d"
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
