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
  version "0.5.0"
  license "MIT"

  depends_on "postgresql@16"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.5.0/chakramcp-server-0.5.0-aarch64-apple-darwin.tar.gz"
      sha256 "d44901834722654854a8a5f3ad26eb4df5eef26613ece25b80264c91b18783a1"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.5.0/chakramcp-server-0.5.0-x86_64-apple-darwin.tar.gz"
      sha256 "a21128ac6585d5b7da9fceb3ca74cff737495c6095b962a1775859246324bb03"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.5.0/chakramcp-server-0.5.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "d6fd35a0233bc5540a08644bb13c3ca0296a003ee5551e3ea045375c797000d1"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.5.0/chakramcp-server-0.5.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "08ed86556ce4bca12c6bf252d48524bf26d52c826a294d9f6d0c20ec4eb774f2"
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
      First-time setup:

        brew services start postgresql@16
        createdb chakramcp
        chakramcp-server init                    # writes server.toml and prints where
        chakramcp-server migrate                 # applies SQL migrations

      Then start it:

        brew services start chakramcp-server     # backgrounds the supervisor
        # — or run in the foreground for logs:
        chakramcp-server start

      Create your account. Public sign-up is closed by default:

        chakramcp-server users add you@example.com --name "Your Name" --admin

      The app service answers on http://localhost:8080 and the relay
      on http://localhost:8090. Point the CLI at it and sign in: your
      browser opens the server's own sign-in page.

        chakramcp networks add private \
          --app-url http://localhost:8080 \
          --relay-url http://localhost:8090
        chakramcp networks use private
        chakramcp login

      Settings live in server.toml: on macOS
      ~/Library/Application Support/com.chakramcp.chakramcp/server.toml,
      on Linux ~/.config/chakramcp/server.toml. Upgrading from 0.2.0?
      Delete its line frontend_base_url = "http://localhost:3000".
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp-server --version")
  end
end
