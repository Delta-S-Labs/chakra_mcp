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
  version "0.4.0"
  license "MIT"

  depends_on "postgresql@16"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.4.0/chakramcp-server-0.4.0-aarch64-apple-darwin.tar.gz"
      sha256 "764c734b020aba99602fef787c88c4da4f3c98f8cae8147bc15fc2233b913521"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.4.0/chakramcp-server-0.4.0-x86_64-apple-darwin.tar.gz"
      sha256 "adb79b9b57d3956bd4c39ad16b9a3c32396796c2050735f587b7c65082b3a616"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.4.0/chakramcp-server-0.4.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "fe422fe48f374bdbbb4b27512a08d4ea56b3b9a889810ca0f1d35b2fa1de0c3a"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.4.0/chakramcp-server-0.4.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "e65e8b264dafec9c94bb0c3fe4f5ccb14a9850bf8216da2e8a63b2e324dece0f"
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
