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
  version "0.1.9"
  license "MIT"

  depends_on "postgresql@16"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.9/chakramcp-server-0.1.9-aarch64-apple-darwin.tar.gz"
      sha256 "9359ef17eec8c1beebba90565f944b1a263a9f883293a94b7b8b5aba154914e7"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.9/chakramcp-server-0.1.9-x86_64-apple-darwin.tar.gz"
      sha256 "0d956c4ea1665e25864830759ecab907e32a919189b6895a872834788223a37c"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.9/chakramcp-server-0.1.9-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "95b10e7b39cf17c75d0254d1fe27bfc3a3c7b2bd3d1a1108a44e768f29b219ee"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.9/chakramcp-server-0.1.9-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "6b4ca7b992ac8363bd610aa6163b57ec9914f90f18e0c73afb516fa6e30cd270"
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
