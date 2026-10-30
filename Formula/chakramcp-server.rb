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
  version "0.7.0"
  license "MIT"

  depends_on "postgresql@16"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.7.0/chakramcp-server-0.7.0-aarch64-apple-darwin.tar.gz"
      sha256 "d395883c1dd4d65601a7549b496e31dd45718a35df8c1403b882bc27964d2448"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.7.0/chakramcp-server-0.7.0-x86_64-apple-darwin.tar.gz"
      sha256 "4b79bbe07235a52301b135e06003ae52a713732020f72aeb5c41f05e14f532e6"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.7.0/chakramcp-server-0.7.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "975e41ca8f5f27806c889d438401143c9f3945ab77a929f272b9d0c690e58657"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.7.0/chakramcp-server-0.7.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "e0fffb9bf677a16802feae63e109f528d9b6cc3596dc97963134ad4dda89f8a7"
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
