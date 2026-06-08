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
  version "0.1.7"
  license "MIT"

  depends_on "postgresql@16"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.7/chakramcp-server-0.1.7-aarch64-apple-darwin.tar.gz"
      sha256 "e140f7edc9984561dcd2eed8b15dff1dfdefaca05cc6d82deae7d09100c979dd"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.7/chakramcp-server-0.1.7-x86_64-apple-darwin.tar.gz"
      sha256 "59b51f3e9e033b46f130d664a951ae6a68bd8cd579593f65755f76dd151ad763"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.7/chakramcp-server-0.1.7-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "6beb66a72d256998908ae5b0d447c1999efc6688f53aeeddfa417d7a9ab90d1e"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.7/chakramcp-server-0.1.7-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "0a8b55ec810eb215b28f874c66145870c3d17de6225196006f152085288a9f75"
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
