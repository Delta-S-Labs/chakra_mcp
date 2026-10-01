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
  version "0.3.0"
  license "MIT"

  depends_on "postgresql@16"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.3.0/chakramcp-server-0.3.0-aarch64-apple-darwin.tar.gz"
      sha256 "133190634e201ef9411d4cebc306f4754eee585f5bd1d73616c1ca738cbea664"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.3.0/chakramcp-server-0.3.0-x86_64-apple-darwin.tar.gz"
      sha256 "3b1a2604d328eb09e9a8a31eab36b9d9e17af5f630b94fc22182e6a8680f58a8"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.3.0/chakramcp-server-0.3.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "c81bf9d8a78299be83427a0a5d2b8910e91df415504a3bf16534329c4b9dfe8b"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.3.0/chakramcp-server-0.3.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "943232fad8a292e3a674e35c5d478913c91f60722f4c81502fd18c2f607edc8d"
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
